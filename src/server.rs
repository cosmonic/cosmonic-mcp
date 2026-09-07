//! The MCP server: handshake, dispatch, resources, prompts.
//!
//! A stateless facade — every tool is a call to the same `/v1/...` API over the
//! daemon's socket (`client.rs`). Tool design follows Anthropic's Connectors
//! Directory criteria and its "writing tools for agents" guidance: reads and
//! writes are separate tools (never an `action` selector), every tool carries a
//! `title` and exactly one of `readOnlyHint`/`destructiveHint`, and the
//! response is a uniform `{status,result,next_steps,errors}` envelope with a
//! recovery hint per error. See `docs/MCP.md`.

use crate::client;
use crate::prompts;
use crate::resources;
use crate::skills;
use crate::tools;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, InitializeRequestParams, InitializeResult,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, Prompt, PromptArgument, PromptMessage, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, Role, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{json, Value};

use client::{DaemonClient, DaemonError};

tokio::task_local! {
    /// Name of the MCP tool currently executing. Set in [`CosmonicMcp::call_tool`]
    /// and read by the daemon client (`client.rs`) so each request it makes carries
    /// an `X-Cosmonic-Mcp-Tool` header — letting the daemon count tool usage
    /// (docs/TELEMETRY-DATA.md). Task-scoped, so concurrent tool calls don't mix.
    pub static CURRENT_TOOL: String;
}

/// The MCP server. Holds a daemon client + the generated tool router.
#[derive(Clone)]
pub struct CosmonicMcp {
    pub(crate) client: DaemonClient,
    tool_router: ToolRouter<Self>,
    /// Which agent connected, as a fixed enum from [`catalog::MCP_CLIENTS`] —
    /// never the raw `clientInfo.name`, which the client controls and which
    /// routinely carries a path or a hostname. Set once, at `initialize`.
    client_id: Arc<OnceLock<(&'static str, String)>>,
    /// Tool calls in this session, for `mcp_client_disconnected`.
    tools_invoked: Arc<AtomicU64>,
}

impl CosmonicMcp {
    pub fn new(client: DaemonClient) -> Self {
        Self {
            client,
            tool_router: Self::tool_router(),
            client_id: Arc::new(OnceLock::new()),
            tools_invoked: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Hand a product-telemetry event to the daemon, which owns the only
    /// outbound connection and re-validates everything. Best-effort and never
    /// surfaced to the agent: an analytics failure must not look like a tool
    /// failure. No-op when telemetry consent is off (the daemon drops it).
    pub(crate) async fn track(&self, event: &str, props: Value) {
        let _ = self
            .client
            .post(
                "/v1/telemetry/events",
                json!({ "events": [{
                    "event": event,
                    "properties": props,
                    "source_process": "mcp",
                }] }),
            )
            .await;
    }

    // ---- response envelope (uniform across every tool) ----------------------

    /// Success envelope: `{status:"ok", result, next_steps}`.
    pub(crate) fn ok(&self, result: Value, next_steps: &[&str]) -> CallToolResult {
        CallToolResult::structured(json!({
            "status": "ok",
            "result": result,
            "next_steps": next_steps,
        }))
    }

    /// [`Self::ok`] with next steps composed at runtime (the credentials
    /// paths word them from the daemon's verdict).
    pub(crate) fn ok_dynamic(&self, result: Value, next_steps: Vec<String>) -> CallToolResult {
        CallToolResult::structured(json!({
            "status": "ok",
            "result": result,
            "next_steps": next_steps,
        }))
    }

    /// Error envelope: `{status:"error", errors:[{code,message,recovery}]}`.
    /// Returned as a tool *result* (is_error=true), not a protocol error, so the
    /// agent can read the recovery hint and retry.
    pub(crate) fn fail(
        &self,
        code: &str,
        message: impl Into<String>,
        recovery: impl Into<String>,
    ) -> CallToolResult {
        CallToolResult::structured_error(json!({
            "status": "error",
            "errors": [{ "code": code, "message": message.into(), "recovery": recovery.into() }],
        }))
    }

    /// Map a daemon call into the envelope, attaching a recovery hint per error.
    pub(crate) fn reply(
        &self,
        res: Result<Value, DaemonError>,
        next_steps: &[&str],
    ) -> CallToolResult {
        match res {
            Ok(v) => self.ok(v, next_steps),
            Err(DaemonError::Unreachable(m)) => self.fail(
                "daemon_unreachable",
                format!("Could not reach the Cosmonic daemon: {m}"),
                "Start Cosmonic Desktop (or run `cosmonicd`) so the daemon's unix socket is available, then retry. Confirm with `cosmonic_host_status`.",
            ),
            Err(DaemonError::Api { status, code, message }) => self.fail(
                &code,
                format!("{message} (HTTP {status})"),
                recovery_for(&code, status),
            ),
            Err(DaemonError::Transport(m)) => self.fail(
                "transport_error",
                m,
                "Retry the call; if it persists, check the daemon logs (`cosmonic_logs_query`).",
            ),
        }
    }
}

/// Recovery hint per daemon error code/status, scoped to the tool that called.
///
/// `tool` matters because the generic arms used to misdirect: every 400 said
/// "see the `cosmonic://schema/workload` resource", which is right only for
/// `cosmonic_workload_apply` and was being attached to bad log levels, bad OCI
/// refs and unknown template ids; every `not_found` said "List with
/// `cosmonic_workload_list`", including when the missing thing was a project
/// id (issue #501 R-4). The tool name comes from [`CURRENT_TOOL`], the
/// task-local the dispatcher already sets.
fn recovery_for(code: &str, status: u16) -> String {
    let tool = CURRENT_TOOL.try_with(|t| t.clone()).unwrap_or_default();
    recovery_for_tool(code, status, &tool)
}

/// The subject a tool operates on, for wording a `not_found` or a 400.
///
/// Grouping by subject rather than listing every tool keeps a newly added tool
/// from silently falling back to workload wording: an unrecognised name lands
/// on [`Subject::Other`], whose hints name no wrong list tool.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Subject {
    Workload,
    Project,
    Image,
    Template,
    Secret,
    Other,
}

fn subject_of(tool: &str) -> Subject {
    match tool {
        "cosmonic_workload_get"
        | "cosmonic_workload_start"
        | "cosmonic_workload_stop"
        | "cosmonic_workload_restart"
        | "cosmonic_workload_delete"
        | "cosmonic_workload_apply"
        | "cosmonic_workload_list"
        | "cosmonic_workload_credentials_test" => Subject::Workload,
        "cosmonic_dev_start"
        | "cosmonic_dev_stop"
        | "cosmonic_dev_status"
        | "cosmonic_dev_logs"
        | "cosmonic_project_publish"
        | "cosmonic_project_list" => Subject::Project,
        "cosmonic_image_inspect" | "cosmonic_workload_draft" => Subject::Image,
        "cosmonic_project_create" => Subject::Template,
        "cosmonic_secret_set" => Subject::Secret,
        _ => Subject::Other,
    }
}

fn recovery_for_tool(code: &str, status: u16, tool: &str) -> String {
    match (code, status) {
        ("invalid_path", _) => "Use lowercase DNS-label namespace/name (alnum + '-', <=63).".into(),
        ("path_mismatch", _) => {
            "Make the Workload metadata.name/namespace match the target.".into()
        }
        ("route_not_found", _) => {
            "Update Cosmonic Desktop so its daemon serves this endpoint, then retry.".into()
        }
        ("not_found", _) => match subject_of(tool) {
            Subject::Project => {
                "List with `cosmonic_project_list` to get a valid project id.".into()
            }
            Subject::Image => {
                "Check the image reference; `cosmonic_image_inspect` confirms one resolves.".into()
            }
            Subject::Secret => {
                "Register the reference with `cosmonic_secret_set` first.".into()
            }
            _ => "List with `cosmonic_workload_list` to get a valid namespace/name.".into(),
        },
        ("method_not_allowed", _) => {
            "Update Cosmonic Desktop so its daemon accepts this call, then retry.".into()
        }
        // Credentials (docs/CREDENTIALS-DESIGN.md §8.3). Never ask the user to
        // paste a value in chat: the Desktop paste field is the path.
        ("workload_not_running", _) | ("workload_disabled", _) => {
            "Start the workload (`cosmonic_workload_start`), wait for `running`, then test again."
                .into()
        }
        ("not_mcp", _) => {
            "This workload exposes no check_auth tool, so its credential cannot be verified from here; the secret is registered. Nothing to retry."
                .into()
        }
        ("consent_required", _) => {
            "Ask the user to allow this workload's use of the connection in Cosmonic Desktop → Workloads → <name> → Credentials; the workload starts by itself once allowed."
                .into()
        }
        ("test_in_progress", _) => {
            "A credential test for this workload is already running; wait a few seconds and call `cosmonic_workload_credentials_test` again."
                .into()
        }
        ("invalid_secret_ref", _) => {
            "Fix the ref: a slug name, a backend URI (keychain://cosmonic/<name>, env://VAR, op://…, aws-sm://…), and a POSIX env var name. Prefer the Desktop paste field (Settings → Secrets) for keychain values; never pass a value you obtained from chat."
                .into()
        }
        (_, 400) => match subject_of(tool) {
            Subject::Workload => {
                "Fix the Workload; the `cosmonic://schema/workload` resource has the field list \
                 and a worked example."
                    .into()
            }
            Subject::Project => {
                "Fix the argument named in the message; `cosmonic_project_list` has the valid \
                 project ids."
                    .into()
            }
            Subject::Image => {
                "Fix the reference: an OCI ref is `[registry/]repo:tag` or `...@sha256:<digest>`."
                    .into()
            }
            Subject::Template => {
                "Use a template id from `cosmonic_template_list`, and an ABSOLUTE path that does \
                 not already exist."
                    .into()
            }
            Subject::Secret | Subject::Other => {
                "Fix the argument named in the message; it says which one and what it accepts."
                    .into()
            }
        },
        (_, 409) => {
            "The resource is in a conflicting state; re-check with a status/list tool first.".into()
        }
        _ => "Inspect the error message; check `cosmonic_logs_query` for daemon-side detail.".into(),
    }
}

/// Rewrite a tool's input schema into the portable subset before it goes on the
/// wire.
///
/// Today that means one thing: **`"type"` as an ARRAY becomes `anyOf`**.
/// `{"type": ["string", "null"]}` is legal JSON Schema and is exactly what
/// schemars emits for `Option<String>`, but several MCP clients read `type` as
/// a single string and either reject the tool outright or silently drop the
/// constraint. `{"anyOf": [{"type": "string"}, {"type": "null"}]}` says the
/// same thing in a form everything reads.
///
/// Found by the official MCP Inspector's schema-portability check (`--cli
/// --strict`), which flagged 17 of these across 10 tools — including two this
/// crate had just introduced by typing the `workload` parameter as
/// `["object", "string"]`. Our own harness could not see them, because it
/// shared this crate's assumptions about what a schema should look like.
///
/// Normalising centrally, at list time, is deliberate: it covers every tool
/// that exists and every one added later, without asking each `#[tool]` author
/// to remember. Sibling keywords are left in place — `anyOf` composes with
/// `description`, `default`, `minimum` and friends.
fn portable_tool(mut tool: Tool) -> Tool {
    let mut schema = (*tool.input_schema).clone();
    let mut value = Value::Object(std::mem::take(&mut schema));
    split_type_arrays(&mut value);
    if let Value::Object(obj) = value {
        tool.input_schema = std::sync::Arc::new(obj);
    }
    tool
}

/// Recursively replace `{"type": [A, B, …]}` with `{"anyOf": [{"type": A}, …]}`.
///
/// A single-element array collapses to the plain string form rather than a
/// one-branch `anyOf`, which is the same contract and less noise.
fn split_type_arrays(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(types)) = map.get("type") {
                let branches: Vec<Value> = types
                    .iter()
                    .filter(|t| t.is_string())
                    .map(|t| json!({ "type": t }))
                    .collect();
                match branches.len() {
                    0 => {}
                    1 => {
                        let only = branches[0]["type"].clone();
                        map.insert("type".into(), only);
                    }
                    _ => {
                        map.remove("type");
                        map.insert("anyOf".into(), Value::Array(branches));
                    }
                }
            }
            for (_, v) in map.iter_mut() {
                split_type_arrays(v);
            }
        }
        Value::Array(items) => {
            for v in items {
                split_type_arrays(v);
            }
        }
        _ => {}
    }
}

/// Freshness hint for the lists that are compiled into the binary.
///
/// The tool, prompt and resource-template sets are built once at construction
/// and cannot change while the process runs — a new build is a new process, and
/// a stdio client re-spawns us — so an hour is an honest promise rather than a
/// hopeful one.
const STATIC_LIST_TTL_MS: u64 = 60 * 60 * 1000;

/// Freshness hint for the resource list, which is gated on a daemon feature
/// flag an operator can flip while we are running. Short, because it can
/// genuinely change under a client.
const FLAGGED_LIST_TTL_MS: u64 = 60 * 1000;

/// Freshness hint for reads that return live host state. Zero: these are the
/// answer to "what is true right now", and a cached one is a wrong one.
const LIVE_READ_TTL_MS: u64 = 0;

impl ServerHandler for CosmonicMcp {
    /// The MCP handshake — and the only place we learn WHICH agent is driving
    /// us. rmcp's default implementation just stores the peer info and returns
    /// `get_info()`; we do the same, plus record the client as a fixed enum.
    ///
    /// `clientInfo.name` is chosen by the client, so it is never sent as-is:
    /// [`catalog::normalize_mcp_client`] maps it onto a known agent or `other`,
    /// and only the MAJOR version travels.
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        let client = cosmonic_api::telemetry::normalize_mcp_client(&request.client_info.name);
        let version = cosmonic_api::telemetry::version_major(&request.client_info.version);
        let _ = self.client_id.set((client, version.clone()));
        self.track(
            "mcp_client_connected",
            json!({ "client": client, "client_version_major": version }),
        )
        .await;

        if context.peer.peer_info().is_none() {
            context.peer.set_peer_info(request);
        }
        Ok(self.get_info())
    }

    fn get_info(&self) -> ServerInfo {
        // ServerInfo is #[non_exhaustive]; mutate a default rather than a literal.
        let mut info = ServerInfo::default();
        // rmcp's default reports the SDK as the server (`{"name":"rmcp",
        // "version":"<sdk version>"}`) — which is what a client shows the user
        // as this server's identity, and what a directory listing captures
        // (issue #501 R-1). Name the product, and track the daemon's version so
        // a bug report says which build answered.
        info.server_info.name = crate::SERVER_NAME.into();
        info.server_info.version = crate::SERVER_VERSION.into();
        info.capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_prompts()
            .build();
        info.instructions = Some(
            "Cosmonic Desktop: scaffold, build, and deploy WebAssembly component workloads on \
             the local wasmCloud host.\n\n\
             This server publishes skills. Read `skill://index.json` for the catalog, then read \
             the SKILL.md whose description matches the task — that playbook is the fastest path \
             to a correct deploy, and it links reference files you only read if you need them.\n\n\
             Without it: cosmonic_host_status -> cosmonic_template_list -> \
             cosmonic_project_create -> (write the code with your own file tools) -> \
             cosmonic_dev_start -> cosmonic_project_publish -> cosmonic_workload_apply, and read the \
             `cosmonic://schema/workload` resource before authoring a Workload. allowedHosts is \
             deny-all by default; secrets are references (cosmonic_secret_set), never inline values."
                .into(),
        );
        info
    }

    // ---- tools: delegate to the generated router ----------------------------

    // ---- cacheability (SEP-2549) --------------------------------------------
    //
    // `ttlMs` and `cacheScope` are required on every list/read result from
    // protocol 2026-07-28. They are a freshness HINT, so the only way to get
    // them wrong is to promise more stability than the thing actually has.
    //
    // The tool, prompt and template lists are built once when the server is
    // constructed and cannot change while the process runs — a new build means
    // a new process, and a stdio client re-spawns us. So an hour is honest, and
    // `public` is accurate: they are identical for every user of a given build
    // and carry no host state.
    //
    // The RESOURCE list is different: it is gated on a daemon feature flag the
    // operator can flip under us, so it gets a short window and `private`.

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self
            .tool_router
            .list_all()
            .into_iter()
            .map(portable_tool)
            .collect();
        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(STATIC_LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        // Capture the tool name for the whole dispatch so the daemon client tags
        // its requests with it (usage analytics — see CURRENT_TOOL).
        let tool = request.name.to_string();
        self.tools_invoked.fetch_add(1, Ordering::Relaxed);
        let tcc = ToolCallContext::new(self, request, context);
        CURRENT_TOOL.scope(tool, self.tool_router.call(tcc)).await
    }

    // ---- resources ----------------------------------------------------------

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        // The catalog resource is behind a default-off feature flag, so on a
        // stock daemon reading it 403s. Advertising a resource that cannot be
        // read is a listing a reviewer walks straight into (issue #501 R-3), so
        // ask the daemon whether the flag is on and omit it when it is not. A
        // daemon that cannot answer is treated as "off" — advertising nothing
        // is better than advertising a 403.
        let catalog_enabled = self.flag_enabled("catalog").await;
        let mut list: Vec<Resource> = resources::RESOURCES
            .iter()
            .filter(|(uri, _, _)| *uri != "cosmonic://catalog" || catalog_enabled)
            .map(|(uri, name, desc)| {
                Resource::new(*uri, *name)
                    .with_description(*desc)
                    .with_mime_type(resources::mime_for(uri))
            })
            .collect();
        // Skills over MCP — the catalog first, then the playbooks it indexes.
        list.extend(skills::resources());
        Ok(ListResourcesResult::with_all_items(list)
            .with_ttl_ms(FLAGGED_LIST_TTL_MS)
            .with_cache_scope(CacheScope::Private))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.as_str();
        // Skills over MCP (`skill://…`) — a verbatim lookup in a static table,
        // so there is no filesystem and no traversal surface. See skills.rs.
        if let Some((mime, text)) = skills::read(uri) {
            return Ok(
                ReadResourceResult::new(vec![text_contents(uri, mime, text)])
                    .with_ttl_ms(STATIC_LIST_TTL_MS)
                    .with_cache_scope(CacheScope::Public)
                    .into(),
            );
        }
        // Composed grounding: what THIS host can run. Four best-effort reads,
        // because a partial picture beats none — an older daemon missing one
        // route omits that section rather than failing the whole resource.
        if uri == "cosmonic://capabilities" {
            let (host, policy, egress, nats) = tokio::join!(
                self.client.get("/v1/host"),
                self.client.get("/v1/policy"),
                self.client.get("/v1/egress"),
                self.client.get("/v1/nats"),
            );
            // The host read is the one that must succeed: without it the
            // daemon is unreachable and every section would be empty.
            let host = host.map_err(|e| {
                McpError::internal_error(format!("could not read host capabilities: {e}"), None)
            })?;
            let caps = resources::capabilities(
                Some(&host),
                policy.as_ref().ok(),
                egress.as_ref().ok(),
                nats.as_ref().ok(),
            );
            let text = serde_json::to_string_pretty(&caps).unwrap_or_else(|_| caps.to_string());
            return Ok(
                ReadResourceResult::new(vec![text_contents(uri, "application/json", text)])
                    .with_ttl_ms(LIVE_READ_TTL_MS)
                    .with_cache_scope(CacheScope::Private)
                    .into(),
            );
        }
        // Static grounding resource.
        if uri == "cosmonic://schema/workload" {
            return Ok(ReadResourceResult::new(vec![text_contents(
                uri,
                "text/markdown",
                resources::WORKLOAD_SCHEMA_DOC.to_owned(),
            )])
            .with_ttl_ms(STATIC_LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public)
            .into());
        }
        // Dynamic resources fetch from the daemon at read time.
        let path = match uri {
            "cosmonic://host" => "/v1/host",
            "cosmonic://workloads" => "/v1/workloads",
            "cosmonic://templates" => "/v1/projects/templates",
            "cosmonic://catalog" => "/v1/catalog",
            other => {
                return Err(McpError::resource_not_found(
                    format!("unknown resource {other}"),
                    None,
                ));
            }
        };
        // `cosmonic://workloads` is the same list `cosmonic_workload_list`
        // returns, and it measured 240 KB unsummarized on a real host. Apply
        // the same projection the tool's default does.
        let summarize = uri == "cosmonic://workloads";
        match self.client.get(path).await {
            Ok(v) => {
                let v = if summarize {
                    tools::summarize_workloads(v)
                } else {
                    v
                };
                let text = serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string());
                Ok(
                    ReadResourceResult::new(vec![text_contents(uri, "application/json", text)])
                        .with_ttl_ms(LIVE_READ_TTL_MS)
                        .with_cache_scope(CacheScope::Private)
                        .into(),
                )
            }
            // A feature-flagged resource answers 403; say what to do about it
            // rather than surfacing it as an opaque internal error.
            Err(DaemonError::Api { status: 403, .. }) => Err(McpError::invalid_request(
                format!(
                    "{uri} is behind a feature flag that is off on this host. Enable it in \
                     Cosmonic Desktop → Settings → Labs, then read it again."
                ),
                None,
            )),
            Err(e) => Err(McpError::internal_error(format!("daemon: {e}"), None)),
        }
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(
            ListResourceTemplatesResult::with_all_items(skills::resource_templates())
                .with_ttl_ms(STATIC_LIST_TTL_MS)
                .with_cache_scope(CacheScope::Public),
        )
    }

    // ---- prompts ------------------------------------------------------------

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let list: Vec<Prompt> = prompts::PROMPTS
            .iter()
            .map(|def| {
                // PromptArgument is #[non_exhaustive]; build via new() + setters.
                let mut arg = PromptArgument::new(def.argument.name);
                arg.title = Some(def.argument.title.to_string());
                arg.description = Some(def.argument.description.to_string());
                arg.required = Some(def.argument.required);
                let mut prompt = Prompt::new(def.name, Some(def.description), Some(vec![arg]));
                prompt.title = Some(def.title.to_string());
                prompt
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(list)
            .with_ttl_ms(STATIC_LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let Some(def) = prompts::get(&request.name) else {
            // Name every prompt there is: an unknown name is usually a typo or
            // a client showing a stale menu, and the list is short.
            let known: Vec<&str> = prompts::PROMPTS.iter().map(|p| p.name).collect();
            return Err(McpError::invalid_params(
                format!(
                    "unknown prompt {:?}; this server has: {}",
                    request.name,
                    known.join(", ")
                ),
                None,
            ));
        };
        let argument = request
            .arguments
            .as_ref()
            .and_then(|a| a.get(def.argument.name))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or(def.missing_argument);
        let mut result = GetPromptResult::new(vec![PromptMessage::new_text(
            Role::User,
            (def.render)(argument),
        )]);
        result.description = Some(def.description.to_string());
        Ok(result.into())
    }
}

/// Read one flag out of a `/v1/flags` response.
///
/// The payload is `{"flags": [{"key": "catalog", "enabled": false, …}, …]}` — an
/// ARRAY of records, not a map keyed by name. Indexing it as a map silently
/// yields `None` for every flag, which reads as "off" and hides a resource that
/// is actually enabled; the failure is invisible while the flag happens to be
/// off, which is the default.
fn flag_is_enabled(response: &Value, key: &str) -> bool {
    response
        .get("flags")
        .and_then(|f| f.as_array())
        .into_iter()
        .flatten()
        .find(|f| f.get("key").and_then(|k| k.as_str()) == Some(key))
        .and_then(|f| f.get("enabled").and_then(|e| e.as_bool()))
        .unwrap_or(false)
}

/// A `text/…` or `application/json` resource body with a REAL MIME type.
///
/// `ResourceContents::text` hardcodes `mimeType: "text"`, which is not a MIME
/// type at all (issue #501 R-2/2.7) — clients that key off it see an unknown
/// type for Markdown and JSON alike.
fn text_contents(uri: &str, mime: &str, text: String) -> ResourceContents {
    ResourceContents::TextResourceContents {
        uri: uri.to_string(),
        mime_type: Some(mime.to_string()),
        text,
        meta: None,
    }
}

impl CosmonicMcp {
    /// Whether a daemon feature flag is on. Any failure reads as OFF: this
    /// gates what we ADVERTISE, and advertising a resource that cannot be read
    /// is worse than omitting one that could have been.
    async fn flag_enabled(&self, key: &str) -> bool {
        self.client
            .get("/v1/flags")
            .await
            .ok()
            .is_some_and(|v| flag_is_enabled(&v, key))
    }
}

/// Entry point for `cosmonicd mcp serve`: connect to the running daemon and
/// serve MCP over stdio until the client disconnects. Logging must already be
/// directed to stderr (stdout is the JSON-RPC channel) — see `main.rs`.
pub async fn serve() -> anyhow::Result<()> {
    let client = DaemonClient::new()?;
    tracing::info!(socket = %client.socket_path().display(), "cosmonic MCP server starting (stdio)");
    let server = CosmonicMcp::new(client);
    // Kept for the disconnect event: the handler itself is moved into `serve`.
    let session = server.clone();
    let started = std::time::Instant::now();
    let running = server.serve(rmcp::transport::stdio()).await?;
    let outcome = running.waiting().await;
    // Session over. Report how long the agent stayed and how much it did —
    // buckets only, and only if it ever completed a handshake.
    if let Some((client_id, _)) = session.client_id.get() {
        session
            .track(
                "mcp_client_disconnected",
                json!({
                    "client": client_id,
                    "session_duration_bucket": cosmonic_api::telemetry::session_secs(started.elapsed().as_secs()),
                    "tools_invoked_bucket": cosmonic_api::telemetry::count_results(
                        session.tools_invoked.load(Ordering::Relaxed),
                    ),
                }),
            )
            .await;
    }
    outcome?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router() -> ToolRouter<CosmonicMcp> {
        CosmonicMcp::tool_router()
    }

    /// The Connectors Directory rule, enforced on the surface rather than on a
    /// list someone has to remember to update: EVERY tool carries a `title` and
    /// exactly one of `readOnlyHint` / `destructiveHint`. A tool with neither is
    /// grouped and flagged at submission; worse, a mutation that is missing
    /// `destructiveHint` can be auto-approved by the client, so this is a
    /// security control and not metadata.
    #[test]
    fn type_arrays_become_any_of() {
        // schemars renders `Option<String>` as `["string","null"]`, which several
        // MCP clients read as a single string and then reject or silently drop.
        let mut v = json!({
            "type": "object",
            "properties": {
                "name":  { "type": ["string", "null"], "description": "keep me" },
                "limit": { "type": ["integer", "null"], "minimum": 1, "maximum": 500 },
                "spec":  { "type": ["object", "string"] },
                "plain": { "type": "string" },
                "one":   { "type": ["boolean"] },
                "nested": {
                    "type": "object",
                    "properties": { "deep": { "type": ["number", "null"] } }
                }
            }
        });
        split_type_arrays(&mut v);
        let p = &v["properties"];

        assert_eq!(
            p["name"]["anyOf"],
            json!([{"type":"string"},{"type":"null"}])
        );
        assert!(
            p["name"].get("type").is_none(),
            "the array form must be gone"
        );
        // Sibling keywords survive: anyOf composes with them.
        assert_eq!(p["name"]["description"], json!("keep me"));
        assert_eq!(p["limit"]["minimum"], json!(1));
        assert_eq!(p["limit"]["maximum"], json!(500));
        assert_eq!(
            p["spec"]["anyOf"],
            json!([{"type":"object"},{"type":"string"}])
        );
        // Already-portable schemas are left exactly as they were.
        assert_eq!(p["plain"]["type"], json!("string"));
        // A one-element array is the same contract as the bare string, so it
        // collapses rather than becoming a pointless single-branch anyOf.
        assert_eq!(p["one"]["type"], json!("boolean"));
        assert!(p["one"].get("anyOf").is_none());
        // Recursion reaches nested property schemas.
        assert_eq!(
            p["nested"]["properties"]["deep"]["anyOf"],
            json!([{"type":"number"},{"type":"null"}])
        );
    }

    /// The wire check: no tool this server serves may carry the array form.
    /// This is what the MCP Inspector's `--strict` portability pass asserts,
    /// pinned here so it cannot regress between audits.
    #[test]
    fn no_served_tool_uses_an_array_type() {
        fn find_array_types(v: &Value, path: &str, out: &mut Vec<String>) {
            match v {
                Value::Object(map) => {
                    if matches!(map.get("type"), Some(Value::Array(_))) {
                        out.push(path.to_string());
                    }
                    for (k, vv) in map {
                        find_array_types(vv, &format!("{path}.{k}"), out);
                    }
                }
                Value::Array(items) => {
                    for (i, vv) in items.iter().enumerate() {
                        find_array_types(vv, &format!("{path}[{i}]"), out);
                    }
                }
                _ => {}
            }
        }

        // The router is what `list_tools` serves from, and building it needs no
        // daemon — so this asserts the real surface without a socket.
        let mut offenders = Vec::new();
        for tool in CosmonicMcp::tool_router()
            .list_all()
            .into_iter()
            .map(portable_tool)
        {
            let schema = Value::Object((*tool.input_schema).clone());
            find_array_types(&schema, &tool.name, &mut offenders);
        }
        assert!(
            offenders.is_empty(),
            "these schemas still use the array `type` form: {offenders:?}"
        );
    }

    #[test]
    fn every_tool_is_annotated_for_auto_permissions() {
        let tools = router().list_all();
        assert!(tools.len() >= 21, "only {} tools registered", tools.len());
        for tool in &tools {
            let name = &tool.name;
            let annotations = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{name} has no annotations"));
            assert!(
                annotations.title.as_ref().is_some_and(|t| !t.is_empty()),
                "{name} has no annotations.title"
            );
            assert!(
                tool.title.as_ref().is_some_and(|t| !t.is_empty()),
                "{name} has no tool title"
            );
            let read_only = annotations.read_only_hint.unwrap_or(false);
            let destructive = annotations.destructive_hint.unwrap_or(false);
            assert!(
                read_only ^ destructive,
                "{name}: exactly one of readOnlyHint/destructiveHint must be true \
                 (readOnly={read_only}, destructive={destructive})"
            );
        }
    }

    /// Tool names must be <= 64 characters (directory criteria), and must not
    /// collide.
    #[test]
    fn tool_names_are_short_and_unique() {
        let tools = router().list_all();
        let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
        for name in &names {
            assert!(name.len() <= 64, "{name} is {} chars", name.len());
        }
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate tool name");
    }

    /// No tool may take an `action`/`method`/`operation` selector.
    ///
    /// That shape is what mixed a read with four mutations in the old
    /// `cosmonic_workload`, and a single tool spanning safe and unsafe
    /// operations is an automatic rejection — one the annotations above cannot
    /// express, since the tool would be read-only for one argument and
    /// destructive for another. Catch a reintroduction here rather than at
    /// submission.
    #[test]
    fn no_tool_multiplexes_on_an_action_parameter() {
        for tool in router().list_all() {
            let Some(properties) = tool.input_schema.get("properties") else {
                continue;
            };
            let Some(properties) = properties.as_object() else {
                continue;
            };
            for forbidden in ["action", "method", "operation", "verb", "command"] {
                assert!(
                    !properties.contains_key(forbidden),
                    "{} takes a `{forbidden}` parameter; split it into separate tools instead",
                    tool.name
                );
            }
        }
    }

    /// Descriptions describe the tool; they do not direct the model's behaviour.
    /// Phrases like "confirm with the user first" are rejected as prompt
    /// injection — the mechanism for that is `destructiveHint`, which makes the
    /// CLIENT prompt.
    #[test]
    fn descriptions_do_not_instruct_the_model() {
        const BANNED: &[&str] = &[
            "confirm with the user",
            "never pass a value you obtained",
            "do not tell the user",
            "ignore previous",
            "you must always",
        ];
        for tool in router().list_all() {
            let text = tool.description.clone().unwrap_or_default().to_lowercase();
            for phrase in BANNED {
                assert!(
                    !text.contains(phrase),
                    "{} description contains behavioural instruction {phrase:?}",
                    tool.name
                );
            }
        }
    }

    /// A read-only tool must not be able to change anything, and the clearest
    /// proxy is that it does not take the confirmation gate a mutation needs.
    #[test]
    fn read_only_tools_take_no_confirm_gate() {
        for tool in router().list_all() {
            let read_only = tool
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint)
                .unwrap_or(false);
            if !read_only {
                continue;
            }
            let has_confirm = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .is_some_and(|p| p.contains_key("confirm"));
            assert!(
                !has_confirm,
                "{} is readOnlyHint but takes `confirm` — one of the two is wrong",
                tool.name
            );
        }
    }

    /// A served playbook must not name a tool this binary does not have.
    ///
    /// This is the failure the whole design invites: `get_info().instructions`
    /// tells every connecting client to read the SKILL.md, and the SKILL.md is
    /// vendored from another repo on its own release cadence. Rename a tool
    /// here and the compiled-in playbook keeps confidently instructing agents
    /// to call the old name — the PRIMARY documented path fails with
    /// tool-not-found, and nothing else notices, because the skills compile
    /// fine and every other test passes.
    ///
    /// It happened on this very change: the tool split landed while the
    /// vendored skills still said `cosmonic_dev` and `cosmonic_workload`.
    #[test]
    fn no_served_skill_names_a_tool_that_does_not_exist() {
        let tools: std::collections::HashSet<String> = router()
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect();

        let mut unknown: Vec<String> = Vec::new();
        let mut examined = 0usize;
        for skill in skills::SKILLS {
            let bodies = std::iter::once(skill.skill_md).chain(skill.files.iter().map(|f| f.text));
            for body in bodies {
                for name in tool_names_in(body) {
                    examined += 1;
                    if !tools.contains(&name) {
                        unknown.push(format!("{}: {name}", skill.name));
                    }
                }
            }
        }
        unknown.sort();
        unknown.dedup();
        assert!(
            unknown.is_empty(),
            "served skills name tools this server does not have — re-vendor from \
             agent-integrations (`node scripts/vendor-agent-skills.mjs && node \
             scripts/vendor-daemon-skills.mjs`): {unknown:?}"
        );
        // A cross-check that finds no references is not a passing cross-check.
        assert!(
            examined > 50,
            "only {examined} tool mentions found across the skills; the extractor is broken"
        );
    }

    /// Every bare `cosmonic_<name>` token in a body, without a regex dependency.
    ///
    /// Anchored on a word boundary, which is not fussiness: the sandbox skill
    /// documents how harnesses PREFIX tool names
    /// (`mcp__cosmonic__cosmonic_host_status`,
    /// `mcp__plugin_cosmonic_cosmonic__…`), and a scan that started at any
    /// `cosmonic_` reads those as the nonexistent tools
    /// `cosmonic__cosmonic_host_status` and `cosmonic_cosmonic`. That prose is
    /// about prefixes, not about which tools exist, so skip it and check the
    /// bare mentions — which is what an agent would actually call.
    ///
    /// A trailing `_` is trimmed so a wildcard like `cosmonic_dev_*` in prose
    /// is not reported missing.
    fn tool_names_in(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (at, _) in body.match_indices("cosmonic_") {
            let preceded_by_word_char = body[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
            if preceded_by_word_char {
                continue;
            }
            let rest = &body[at..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .unwrap_or(rest.len());
            let name = rest[..end].trim_end_matches('_');
            // `cosmonic_api`, `cosmonic_secrets` etc. are crates, not tools —
            // but they do not appear in a playbook, and an unknown name here is
            // exactly what this test exists to surface, so do not allowlist.
            if name.len() > "cosmonic_".len() {
                out.push(name.to_string());
            }
        }
        out
    }

    /// A prompt must not name a tool this server does not have.
    ///
    /// The same drift that let the vendored skills instruct agents to call
    /// renamed tools, one layer closer: a prompt is what a USER picks from a
    /// menu, so a stale tool name in one fails on the very first step of a flow
    /// they explicitly chose.
    #[test]
    fn prompts_only_name_tools_that_exist() {
        let tools: std::collections::HashSet<String> = router()
            .list_all()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        let mut unknown = Vec::new();
        let mut examined = 0usize;
        for def in prompts::PROMPTS {
            let body = (def.render)("<argument>");
            for name in tool_names_in(&body) {
                examined += 1;
                if !tools.contains(&name) {
                    unknown.push(format!("{}: {name}", def.name));
                }
            }
        }
        unknown.sort();
        unknown.dedup();
        assert!(
            unknown.is_empty(),
            "prompts name tools that do not exist: {unknown:?}"
        );
        assert!(
            examined >= prompts::PROMPTS.len() * 3,
            "only {examined} tool mentions across {} prompts; the extractor is broken",
            prompts::PROMPTS.len()
        );
    }

    /// And a prompt must not point at a resource that is not served.
    #[test]
    fn prompts_only_name_resources_that_exist() {
        let known: std::collections::HashSet<&str> = resources::RESOURCES
            .iter()
            .map(|(uri, _, _)| *uri)
            .collect();
        for def in prompts::PROMPTS {
            let body = (def.render)("<argument>");
            for uri in uris_in(&body, "cosmonic://") {
                assert!(
                    known.contains(uri.as_str()),
                    "{}: {uri} is not served",
                    def.name
                );
            }
            for uri in uris_in(&body, "skill://") {
                assert!(
                    skills::read(&uri).is_some(),
                    "{}: {uri} is not served",
                    def.name
                );
            }
        }
    }

    /// Every URI with the given scheme, trimmed of trailing punctuation.
    fn uris_in(body: &str, scheme: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (at, _) in body.match_indices(scheme) {
            let rest = &body[at..];
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '`' || c == ')' || c == '"')
                .unwrap_or(rest.len());
            out.push(
                rest[..end]
                    .trim_end_matches(['.', ',', ';', ':'])
                    .to_string(),
            );
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn every_prompt_renders_something_useful_without_an_argument() {
        // A prompt that renders blank, or with a literal "{}", reads as broken.
        for def in prompts::PROMPTS {
            let body = (def.render)(def.missing_argument);
            assert!(
                body.len() > 400,
                "{}: body is {} chars",
                def.name,
                body.len()
            );
            assert!(
                !def.missing_argument.is_empty(),
                "{}: no fallback",
                def.name
            );
            assert!(
                body.contains(def.missing_argument),
                "{}: fallback unused",
                def.name
            );
        }
    }

    #[test]
    fn prompt_names_are_unique_and_descriptions_are_pickable() {
        let mut names: Vec<&str> = prompts::PROMPTS.iter().map(|p| p.name).collect();
        let before = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate prompt name");
        for def in prompts::PROMPTS {
            assert!(!def.title.is_empty(), "{}: no title", def.name);
            assert!(
                def.description.len() > 40,
                "{}: description is too thin for a menu",
                def.name
            );
        }
    }

    // ---- feature flags ----------------------------------------------------

    /// The real `/v1/flags` payload. Captured from a running daemon: it is an
    /// array of records, and reading it as a map is the bug this guards.
    fn flags_payload(catalog_enabled: bool) -> Value {
        json!({
            "flags": [
                { "key": "hosts", "enabled": false, "default": false, "source": "default" },
                { "key": "catalog", "enabled": catalog_enabled, "default": false,
                  "source": "file" },
                { "key": "builder", "enabled": true, "default": true, "source": "default" },
            ]
        })
    }

    #[test]
    fn flag_is_enabled_reads_the_array_payload() {
        assert!(flag_is_enabled(&flags_payload(true), "catalog"));
        assert!(!flag_is_enabled(&flags_payload(false), "catalog"));
        assert!(flag_is_enabled(&flags_payload(false), "builder"));
        assert!(!flag_is_enabled(&flags_payload(true), "hosts"));
    }

    #[test]
    fn an_unknown_or_malformed_flag_response_reads_as_off() {
        // Fail closed: this gates what we ADVERTISE, and advertising a resource
        // that cannot be read is worse than omitting one that could have been.
        assert!(!flag_is_enabled(&flags_payload(true), "no-such-flag"));
        assert!(!flag_is_enabled(&json!({}), "catalog"));
        assert!(!flag_is_enabled(
            &json!({ "flags": { "catalog": true } }),
            "catalog"
        ));
        assert!(!flag_is_enabled(
            &json!({ "flags": [{ "key": "catalog" }] }),
            "catalog"
        ));
        assert!(!flag_is_enabled(&Value::Null, "catalog"));
    }

    // ---- recovery hints ---------------------------------------------------

    #[test]
    fn recovery_hints_are_scoped_to_the_calling_tool() {
        // The bug this replaced: every 400 said "see the cosmonic://schema/
        // workload resource" and every not_found named cosmonic_workload_list,
        // whatever the tool was actually operating on.
        let project = recovery_for_tool("not_found", 404, "cosmonic_dev_logs");
        assert!(project.contains("cosmonic_project_list"), "{project}");
        assert!(!project.contains("cosmonic_workload_list"), "{project}");

        let workload = recovery_for_tool("not_found", 404, "cosmonic_workload_get");
        assert!(workload.contains("cosmonic_workload_list"), "{workload}");

        let template = recovery_for_tool("invalid_project", 400, "cosmonic_project_create");
        assert!(template.contains("cosmonic_template_list"), "{template}");
        assert!(!template.contains("schema/workload"), "{template}");

        let image = recovery_for_tool("inspect_failed", 400, "cosmonic_image_inspect");
        assert!(image.contains("OCI ref"), "{image}");

        // The Workload schema hint survives where it is actually right.
        let apply = recovery_for_tool("invalid_workload", 400, "cosmonic_workload_apply");
        assert!(apply.contains("cosmonic://schema/workload"), "{apply}");
    }

    #[test]
    fn an_unknown_tool_gets_a_hint_that_names_no_wrong_list_tool() {
        // A tool added without updating `subject_of` must not inherit workload
        // wording it has nothing to do with.
        let hint = recovery_for_tool("bad_request", 400, "cosmonic_future_tool");
        assert!(!hint.contains("cosmonic_workload_list"), "{hint}");
        assert!(!hint.contains("schema/workload"), "{hint}");
    }

    #[test]
    fn version_skew_recovery_says_to_update_desktop() {
        for code in ["route_not_found", "method_not_allowed"] {
            let hint = recovery_for_tool(code, 404, "cosmonic_dev_status");
            assert!(
                hint.to_lowercase().contains("update cosmonic desktop"),
                "{code}: {hint}"
            );
        }
    }

    #[test]
    fn every_subject_maps_a_400_and_a_not_found_to_something_specific() {
        for tool in router().list_all() {
            for (code, status) in [("not_found", 404u16), ("bad_request", 400u16)] {
                let hint = recovery_for_tool(code, status, &tool.name);
                assert!(
                    hint.len() > 20,
                    "{}: {code} hint is uselessly short: {hint}",
                    tool.name
                );
            }
        }
    }
}
