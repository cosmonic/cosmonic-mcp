//! The workspace MCP App (`ui://cosmonic/workspace/<digest>`, see [`uri`]); the server's own tools
//! keep their dispatch and annotations.
//!
//! The workspace is Cosmonic Desktop's own renderer (screens, design system and
//! data layer), built by `js/packages/cosmonic-mcp-app` into `ui/index.html`
//! and embedded here as one self-contained MCP App resource. Desktop's renderer reaches the daemon
//! through `window.cosmonic.daemon.request(method, path, body)`; inside the MCP
//! App that bridge is backed by the app-only tools below:
//!
//! - `cosmonic_desktop_daemon_get`: a read-only GET proxy, limited to the
//!   `/v1` routes Desktop's screens read.
//! - `cosmonic_desktop_daemon_write`: the lifecycle and local-registry writes
//!   those screens make (start/stop/restart/delete, image locks, prune...), each
//!   allowlisted by method and route shape.
//! - `cosmonic_desktop_handoff`: opens a `cosmonic://navigate/...` deep link so
//!   Cosmonic Desktop shows a workload in its own MCP Inspector, Inspect or Logs.
//! - `cosmonic_desktop_agents`: Desktop's "Coding agents" switch for the host
//!   this plugin runs in (Codex), writing the same managed block Desktop writes.
//!
//! All four carry `_meta.ui.visibility: ["app"]`, so the host lets the embedded
//! app call them and keeps them out of the model's tool list.
use crate::client::DaemonError;
use crate::{tools, CosmonicMcp};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{
    model::{CallToolResult, ContentBlock, ResourceContents, Tool},
    ErrorData,
};
use serde_json::{json, Map, Value};
/// The unversioned URI: still served, for hosts holding a tool list from
/// before the URI carried a version.
pub const BASE_URI: &str = "ui://cosmonic/workspace";
const HTML: &str = include_str!("../ui/index.html");

/// The workspace's URI: [`BASE_URI`] plus a digest of the embedded HTML, so
/// each UI build has its own address. ChatGPT and Codex cache a template by
/// its URI (OpenAI's Apps SDK guidance is to change the URI to ship a new
/// one); with a fixed URI a rebuilt UI kept showing the previous build.
pub fn uri() -> &'static str {
    static URI: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(HTML.as_bytes());
        let hex: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        format!("{BASE_URI}/{hex}")
    });
    &URI
}

/// Whether `uri` names the workspace: this build's URI, or [`BASE_URI`].
pub fn is_uri(requested: &str) -> bool {
    requested == uri() || requested == BASE_URI
}
pub const MIME: &str = "text/html;profile=mcp-app";
/// The MCP Apps extension id a rendering host declares in its capabilities.
pub const EXTENSION: &str = "io.modelcontextprotocol/ui";

/// Deploy intents Desktop's screens send with an apply (data.js applyWorkload
/// callers), from the daemon's `DEPLOY_SOURCES` (usage/catalog.rs).
const DEPLOY_SOURCES: [&str; 4] = ["template", "local_file", "registry", "vscode"];
const OPEN_TOOLS: [&str; 3] = [
    "cosmonic_desktop_open",
    "cosmonic_desktop_panel",
    "cosmonic_desktop_refresh",
];
const APP_TOOLS: [&str; 6] = [
    "cosmonic_desktop_daemon_get",
    "cosmonic_desktop_daemon_write",
    "cosmonic_desktop_handoff",
    "cosmonic_desktop_agents",
    "cosmonic_desktop_builder",
    "cosmonic_desktop_content",
];

pub fn is_tool(name: &str) -> bool {
    OPEN_TOOLS.contains(&name) || APP_TOOLS.contains(&name)
}

pub fn tools() -> Vec<Tool> {
    let mut out: Vec<Tool> = OPEN_TOOLS.iter().map(|name| {
        let mut meta = json!({"ui":{"resourceUri":uri()}});
        if *name == "cosmonic_desktop_panel" { meta["openai/ui"] = json!({"entrypoints":[{"type":"thread"}]}); }
        if *name == "cosmonic_desktop_open" { meta["openai/ui"] = json!({"entrypoints":[{"type":"global"}]}); }
        serde_json::from_value(json!({
            // `panel` is the thread entry point: Codex lists it in a thread's
            // side-panel launcher, and opens it beside that thread's chat.
            "name":name,"title":if *name == "cosmonic_desktop_refresh" {"Refresh Cosmonic workspace"} else {"Cosmonic Desktop"},
            "description":if *name == "cosmonic_desktop_panel" {
                "Open the local Cosmonic Desktop workspace beside this thread: workloads, Launchpad, Academy, Build and more. Accepts empty arguments. No deployments are changed."
            } else {
                "Open or refresh the local Cosmonic Desktop workspace with verified host, workload, project and template data. Accepts empty arguments. No deployments are changed."
            },
            "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
            "annotations":{"title":if *name == "cosmonic_desktop_refresh" {"Refresh Cosmonic workspace"} else {"Cosmonic Desktop"},
                "readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false},
            "_meta":meta
        })).expect("static tool definition")
    }).collect();
    out.extend([
        app_tool("cosmonic_desktop_daemon_get", "Read Cosmonic daemon state",
            "Workspace-internal: GET allowlisted /v1 daemon routes (one `path`, or a batch of `paths`) for the Cosmonic Desktop workspace UI.",
            json!({"type":"object","properties":{
                "path":{"type":"string","maxLength":2048},
                "paths":{"type":"array","items":{"type":"string","maxLength":2048},"maxItems":16}
            },"additionalProperties":false}),
            json!({"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})),
        app_tool("cosmonic_desktop_daemon_write", "Change a Cosmonic workload or image",
            "Workspace-internal: perform one allowlisted workload lifecycle or local-registry change the user requested in the Cosmonic Desktop workspace UI.",
            json!({"type":"object","properties":{
                "method":{"type":"string","enum":["POST","PUT","DELETE"]},
                "path":{"type":"string","maxLength":2048},
                "body":{"type":["object","array","string","number","boolean","null"],"description":"JSON request body, sent as given."},
                "deploySource":{"type":"string","enum":["template","local_file","registry","vscode"]}
            },"required":["method","path"],"additionalProperties":false}),
            json!({"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false})),
        app_tool("cosmonic_desktop_handoff", "Show in Cosmonic Desktop",
            "Workspace-internal: open a workload in the Cosmonic Desktop app (its MCP Inspector, Inspect graph, Logs or Workloads row), or a local workload URL in the default browser.",
            json!({"type":"object","properties":{
                "route":{"type":"string","enum":["app","settings","mcp-inspector","inspect","logs","workloads","open-url"]},
                "workload":{"type":"string","maxLength":200},
                "url":{"type":"string","maxLength":2048}
            },"required":["route"],"additionalProperties":false}),
            json!({"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})),
        app_tool("cosmonic_desktop_content", "Read Launchpad and Academy content",
            "Workspace-internal: read the Launchpad and Academy catalogs Cosmonic Desktop has installed (index, READMEs, workload files, icons).",
            json!({"type":"object","properties":{
                "action":{"type":"string","enum":["hub","files","env"]},
                "hub":{"type":"string","enum":["launchpad","academy"]},
                "files":{"type":"array","maxItems":64,"items":{"type":"object","properties":{
                    "hub":{"type":"string"},"source":{"type":"string"},"path":{"type":"string","maxLength":1024}
                },"required":["hub","source","path"],"additionalProperties":false}}
            },"required":["action"],"additionalProperties":false}),
            json!({"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})),
        app_tool("cosmonic_desktop_builder", "Launch a coding agent from the Builder",
            "Workspace-internal: the Build page's terminal list, coding-agent detection, free project names, and launching the chosen agent in the user's terminal app. Only an MCP Apps host may call it.",
            json!({"type":"object","properties":{
                "action":{"type":"string","enum":["host","terminals","presence","free-name","launch"]},
                "dir":{"type":"string","maxLength":4096},
                "name":{"type":"string","maxLength":63},
                "terminal":{"type":"string","maxLength":64},
                "cwd":{"type":"string","maxLength":4096},
                "argv":{"type":"array","maxItems":64,"items":{"type":"string"}},
                "env":{"type":"object","additionalProperties":{"type":"string"}},
                "title":{"type":"string","maxLength":400}
            },"required":["action"],"additionalProperties":false}),
            json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":false,"openWorldHint":true})),
        app_tool("cosmonic_desktop_agents", "Manage the MCP server in Codex",
            "Workspace-internal: read, add or remove an MCP-server workload's entry in the Codex config, in the managed block Cosmonic Desktop uses.",
            json!({"type":"object","properties":{
                "action":{"type":"string","enum":["state","configure","unconfigure"]},
                "workload":{"type":"string","maxLength":200},
                "url":{"type":"string","maxLength":2048}
            },"required":["action","workload"],"additionalProperties":false}),
            json!({"readOnlyHint":false,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false})),
    ]);
    out
}

fn app_tool(
    name: &str,
    title: &str,
    description: &str,
    schema: Value,
    mut annotations: Value,
) -> Tool {
    annotations["title"] = json!(title);
    serde_json::from_value(json!({
        "name":name,"title":title,"description":description,"inputSchema":schema,"annotations":annotations,
        "_meta":{"ui":{"resourceUri":uri(),"visibility":["app"]},"openai/widgetAccessible":true,"openai/visibility":"private"}
    })).expect("static tool definition")
}

/// The workspace's contents, under the URI the host asked for (the versioned
/// one, or [`BASE_URI`] from an older tool list).
pub fn resource(requested: &str) -> ResourceContents {
    serde_json::from_value(json!({"uri":requested,"mimeType":MIME,"text":HTML,"_meta":{
        "ui":{"csp":{"connectDomains":[],"resourceDomains":[]},"prefersBorder":false},
        "openai/ui":{"availableDisplayModes":["fullscreen"],"preferredDisplayMode":"fullscreen"}
    }}))
    .expect("static resource definition")
}

pub async fn call(
    server: &CosmonicMcp,
    name: &str,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, ErrorData> {
    let args = args.unwrap_or_default();
    match name {
        "cosmonic_desktop_daemon_get" => {
            // A batch is how the app polls: Desktop's data layer fans out one
            // round of GETs at once, and the app's bridge coalesces them into a
            // single tool call. Each path answers independently.
            if let Some(paths) = args.get("paths").and_then(Value::as_array) {
                let paths: Vec<&str> = paths.iter().filter_map(Value::as_str).collect();
                if paths.is_empty() || paths.len() > 16 {
                    return Err(ErrorData::invalid_params(
                        "`paths` takes 1 to 16 routes",
                        None,
                    ));
                }
                if let Some(bad) = paths.iter().find(|p| !readable(p)) {
                    return Err(ErrorData::invalid_params(
                        format!("GET {bad} is not available to the workspace"),
                        None,
                    ));
                }
                let results = futures_join_all(paths.iter().map(|p| {
                    let (client, p) = (server.client.clone(), p.to_string());
                    async move { client.get(&p).await }
                }))
                .await;
                let results: Vec<Value> = results
                    .into_iter()
                    .map(|r| daemon_result(r).structured_content.unwrap_or(Value::Null))
                    .collect();
                return Ok(app_result(json!({"ok":true,"results":results})));
            }
            let path = str_arg(&args, "path")?;
            if !readable(path) {
                return Err(ErrorData::invalid_params(
                    format!("GET {path} is not available to the workspace"),
                    None,
                ));
            }
            Ok(daemon_result(server.client.get(path).await))
        }
        "cosmonic_desktop_daemon_write" => {
            let method = str_arg(&args, "method")?;
            let path = str_arg(&args, "path")?;
            if !writable(method, path) {
                return Err(ErrorData::invalid_params(
                    format!("{method} {path} is not available to the workspace"),
                    None,
                ));
            }
            let body = args.get("body").cloned().filter(|b| !b.is_null());
            if body.as_ref().is_some_and(carries_secret) {
                return Ok(CallToolResult::structured_error(
                    json!({"ok":false,"status":403,"code":"desktop_only",
                    "message":"Secret values and passwords are entered in the Cosmonic Desktop app, so they never pass through this chat host."}),
                ));
            }
            // These are the user's clicks in the workspace, not the model's
            // tool calls: tag them `ui` like Desktop's own screens, with the
            // deploy intent those screens attach to an apply.
            let deploy_source = args
                .get("deploySource")
                .and_then(Value::as_str)
                .and_then(|d| DEPLOY_SOURCES.iter().copied().find(|k| *k == d));
            Ok(daemon_result(
                server
                    .client
                    .for_ui(deploy_source)
                    .request(method, path, body)
                    .await,
            ))
        }
        "cosmonic_desktop_handoff"
            if args.get("route").and_then(Value::as_str) == Some("open-url") =>
        {
            open_url(str_arg(&args, "url")?)
        }
        "cosmonic_desktop_handoff" => handoff(
            str_arg(&args, "route")?,
            args.get("workload").and_then(Value::as_str),
        ),
        "cosmonic_desktop_content" => Ok(match crate::ui_content::call(&args) {
            Ok(body) => app_result(body),
            Err(message) => CallToolResult::structured_error(json!({"ok":false,"message":message})),
        }),
        "cosmonic_desktop_builder" => builder(server, &args).await,
        "cosmonic_desktop_agents" => agents(
            str_arg(&args, "action")?,
            str_arg(&args, "workload")?,
            args.get("url").and_then(Value::as_str),
        ),
        _ => open(server, Some(args)).await,
    }
}

/// The Builder's main-process half (`builder.rs`). Launching starts a process
/// on the user's machine, so it answers only a client that renders MCP Apps:
/// such a host keeps app-only tools away from its model, while a plain agent
/// client (which lists every tool) would hand this one to the model.
async fn builder(
    server: &CosmonicMcp,
    args: &Map<String, Value>,
) -> Result<CallToolResult, ErrorData> {
    if !server.renders_apps() {
        tracing::info!(client = ?server.client(), "builder refused: client doesn't render apps");
        return Err(ErrorData::invalid_request(
            "the Builder runs only inside an MCP Apps host (the Cosmonic workspace)",
            None,
        ));
    }
    let action = str_arg(args, "action")?;
    tracing::info!(action, client = ?server.client(), "builder");
    Ok(match action {
        // The coding agent whose thread hosts the workspace, if any. The
        // iframe can't tell: Codex's ui/initialize hostInfo is just
        // { name: "chatgpt" }, the same as a plain ChatGPT chat.
        "host" => app_result(json!({
            "threadAgent": (server.client() == Some("codex")).then_some("Codex"),
        })),
        "terminals" => app_result(crate::builder::terminals()),
        "presence" => app_result(crate::builder::presence().await),
        "free-name" => app_result(crate::builder::free_name(
            str_arg(args, "dir")?,
            str_arg(args, "name")?,
        )),
        "launch" => match crate::builder::launch(args).await {
            Ok(v) => app_result(v),
            Err(error) => CallToolResult::structured_error(json!({"ok":false,"error":error})),
        },
        _ => return Err(ErrorData::invalid_params("unknown action", None)),
    })
}

/// Run the futures concurrently and collect their outputs in order (a tiny
/// `join_all`, so the crate takes no `futures` dependency for one call site).
async fn futures_join_all<F>(futures: impl Iterator<Item = F>) -> Vec<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    let handles: Vec<_> = futures.map(tokio::spawn).collect();
    let mut out = Vec::with_capacity(handles.len());
    for h in handles {
        out.push(h.await.expect("daemon read task"));
    }
    out
}

/// Structured content only: the app reads `structuredContent`, and the default
/// text copy of a daemon listing would double every poll's payload.
fn app_result(body: Value) -> CallToolResult {
    let mut result = CallToolResult::structured(body);
    result.content = vec![ContentBlock::text("ok")];
    result
}

fn str_arg<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, ErrorData> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ErrorData::invalid_params(format!("`{key}` is required"), None))
}

/// The daemon's answer in the shape Desktop's preload hands its renderer:
/// the JSON body on success; `{status, code, message}` on failure (or
/// `code: ECONNREFUSED` when the socket is unreachable, which Desktop's data
/// layer reads as "engine down").
fn daemon_result(result: Result<Value, DaemonError>) -> CallToolResult {
    match result {
        Ok(body) => app_result(json!({"ok":true,"body":body})),
        Err(e) => {
            let detail = match &e {
                DaemonError::Api {
                    status,
                    code,
                    message,
                } => json!({"ok":false,"status":status,"code":code,"message":message}),
                DaemonError::Unreachable(m) => {
                    json!({"ok":false,"code":"ECONNREFUSED","message":format!("Cosmonic Desktop's daemon is not reachable: {m}")})
                }
                DaemonError::Transport(m) => json!({"ok":false,"code":"transport","message":m}),
            };
            let mut result = CallToolResult::structured_error(detail);
            result.content = vec![ContentBlock::text(e.to_string())];
            result
        }
    }
}

/// A `/v1` path with no traversal, whitespace or fragments: the shape every
/// allowlisted route shares before its own check.
fn clean_path(path: &str) -> Option<(Vec<&str>, Option<&str>)> {
    if !path.starts_with("/v1/")
        || path.len() > 2048
        || path.contains("..")
        || path.contains('#')
        || path
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return None;
    }
    let (route, query) = match path.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (path, None),
    };
    let segments: Vec<&str> = route[4..].split('/').collect();
    if segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    Some((segments, query))
}

/// The routes Desktop's Workloads, Inspect, Local Registry, Logs, Projects and
/// Capabilities screens read. `/v1/events` (a never-ending SSE stream) is not one
/// of them: the app polls instead.
fn readable(path: &str) -> bool {
    let Some((s, _)) = clean_path(path) else {
        return false;
    };
    matches!(
        s[0],
        "host"
            | "workloads"
            | "oci"
            | "inspect"
            | "logs"
            | "projects"
            | "registries"
            | "configs"
            | "flags"
            | "nats"
            | "kafka"
            | "policy"
            | "egress"
            | "mirrors"
            | "notify"
            | "mcp"
            | "preflight"
    ) || (s[0] == "system" && s.get(1) == Some(&"workloads"))
        || (s[0] == "secrets" && s.get(1) == Some(&"refs"))
        || (s[0] == "config" && s.get(1) == Some(&"revisions"))
}

/// The writes Desktop's reused screens make: workload lifecycle and deploys,
/// the local registry, project scaffolding, secret and config references, and
/// the host settings Desktop's Settings screen edits (all references and
/// policy, never credentials; see [`carries_secret`]).
fn writable(method: &str, path: &str) -> bool {
    let Some((s, query)) = clean_path(path) else {
        return false;
    };
    if query.is_some() {
        return false;
    }
    matches!(
        (method, s.as_slice()),
        ("POST", ["workloads"] | ["synthesize"])
            | ("POST", ["workloads", _, _, "start" | "stop" | "restart"])
            | ("POST", ["workloads", _, _, "credentials", "test"])
            | ("POST", ["workloads", _, _, "revisions", _, "activate"])
            | ("PUT" | "DELETE", ["workloads", _, _])
            | ("POST" | "DELETE", ["oci", _, "lock"])
            | ("POST", ["oci", "prune" | "import" | "export"])
            | ("POST", ["projects", "new"])
            | ("POST", ["projects", _, "dev", "start" | "stop"])
            | ("POST", ["secrets", "refs"])
            | ("DELETE", ["secrets", "refs", _])
            | ("POST", ["configs"])
            | ("DELETE", ["configs", _])
            | ("PUT", ["config", "revisions"])
            | ("DELETE", ["registries", _])
            | ("POST" | "DELETE", ["registries", _, "ca"])
            | (
                "PUT",
                ["host", "config"]
                    | ["mcp", "config"]
                    | ["policy"]
                    | ["mirrors"]
                    | ["egress"]
                    | ["nats"]
                    | ["kafka"]
                    | ["logs", "config"]
                    | ["notify", "config"]
                    | ["notify", "components", _]
            )
            | ("POST", ["nats", "test"] | ["kafka", "test"])
    )
}

/// A secret value or password in a request body. Tool-call arguments travel
/// through the chat host, which may record them, so these stay in the Desktop
/// app: a keychain secret's value, a registry login. References (`op://`,
/// `env://`, `secret_from` names) carry no secret and pass.
fn carries_secret(body: &Value) -> bool {
    body.as_object().is_some_and(|o| {
        ["value", "password", "token"]
            .iter()
            .any(|k| o.get(*k).is_some_and(|v| !v.is_null() && v != ""))
    })
}

/// `<namespace>/<name>` split into the two safe path segments Desktop's
/// deep-link parser accepts (`src/main/navigate.ts` PATH_SEGMENT).
fn workload_key(key: &str) -> Result<(&str, &str), ErrorData> {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 128
            && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    match key.split_once('/') {
        Some((ns, name)) if ok(ns) && ok(name) => Ok((ns, name)),
        _ => Err(ErrorData::invalid_params(
            "`workload` must be <namespace>/<name>",
            None,
        )),
    }
}

fn handoff(route: &str, workload: Option<&str>) -> Result<CallToolResult, ErrorData> {
    // `app` only brings Desktop forward: `cosmonic://navigate` names no route,
    // which Desktop's parser (navigate.js) ignores after focusing the window.
    let url = match (route, workload) {
        ("app", _) => "cosmonic://navigate".to_string(),
        ("settings", _) => "cosmonic://navigate/settings".to_string(),
        ("mcp-inspector" | "inspect" | "logs" | "workloads", Some(w)) => {
            let (ns, name) = workload_key(w)?;
            format!("cosmonic://navigate/{route}/{ns}/{name}")
        }
        _ => {
            return Err(ErrorData::invalid_params(
                "unknown route, or `workload` missing",
                None,
            ))
        }
    };
    launch(url)
}

/// A workload's ingress URL, opened in the default browser by this process.
/// Codex only opens `https:` links an app asks for, and ingress is plain
/// `http:` on a loopback name, so the app's "Open in browser" comes here.
/// Only loopback hosts are accepted: this is for the user's own workloads.
fn open_url(url: &str) -> Result<CallToolResult, ErrorData> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| ErrorData::invalid_params("only http(s) URLs", None))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once(':').map_or(authority, |(h, port)| {
        if port.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            authority
        }
    });
    let local = host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host.ends_with(".localhost")
        || host.ends_with(".localhost.cosmonic.sh");
    if !local || authority.contains('@') || url.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(ErrorData::invalid_params(
            "only local workload URLs are opened",
            None,
        ));
    }
    launch(url.to_string())
}

fn launch(url: String) -> Result<CallToolResult, ErrorData> {
    let spawned = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).spawn()
    } else if cfg!(windows) {
        // Not `cmd /C start`: cmd would split the URL at `&`.
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&url).spawn()
    };
    match spawned.and_then(|mut child| child.wait()) {
        Ok(status) if status.success() => {
            Ok(CallToolResult::structured(json!({"ok":true,"url":url})))
        }
        Ok(status) => Ok(CallToolResult::structured_error(
            json!({"ok":false,"url":url,
            "message":if url.starts_with("cosmonic:") { format!("Cosmonic Desktop did not open ({status}). Is the app installed?") } else { format!("Couldn't open {url} ({status}).") }}),
        )),
        Err(e) => Ok(CallToolResult::structured_error(
            json!({"ok":false,"url":url,"message":e.to_string()}),
        )),
    }
}

// ---- Coding agents (Codex) -------------------------------------------------
// A port of the TOML engine in Desktop's src/main/mcpClients.ts, for the one
// agent this plugin runs inside. Same per-workload markers, same server key
// (`serverKeyFor`), same `.cosmonic.bak` snapshot, so Desktop and the plugin
// read each other's entries as their own.

fn codex_config() -> Option<std::path::PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
        .map(|d| d.join("config.toml"))
}

fn server_key(ns: &str, name: &str) -> String {
    if ns == "default" {
        name.to_string()
    } else {
        format!("{ns}-{name}")
    }
}

fn markers(key: &str) -> (String, String) {
    (
        format!("# >>> cosmonic mcp: {key} (managed by Cosmonic Desktop) >>>"),
        format!("# <<< cosmonic mcp: {key} <<<"),
    )
}

fn has_table(text: &str, key: &str) -> bool {
    let table = format!("[mcp_servers.{key}]");
    text.lines().any(|l| l.trim_start().starts_with(&table))
}

fn strip_block(text: &str, key: &str) -> String {
    let (begin, end) = markers(key);
    let Some(b) = text.find(&begin) else {
        return text.to_string();
    };
    let e = text[b..]
        .find(&end)
        .map_or(text.len(), |i| b + i + end.len());
    let rest = &text[e..];
    let mut out = format!("{}{}", &text[..b], rest.strip_prefix('\n').unwrap_or(rest));
    while out.ends_with("\n\n\n") {
        out.pop();
    }
    out
}

fn agents(action: &str, workload: &str, url: Option<&str>) -> Result<CallToolResult, ErrorData> {
    let (ns, name) = workload_key(workload)?;
    if !(ns.chars().chain(name.chars()))
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(ErrorData::invalid_params(
            "only lowercase workload names can be registered",
            None,
        ));
    }
    let key = server_key(ns, name);
    if key == "cosmonic" {
        return Err(ErrorData::invalid_params(
            "`cosmonic` is reserved for the sandbox server",
            None,
        ));
    }
    let Some(path) = codex_config() else {
        return Ok(CallToolResult::structured_error(
            json!({"ok":false,"message":"No home directory"}),
        ));
    };
    let read = || std::fs::read_to_string(&path).unwrap_or_default();
    let (begin, end) = markers(&key);
    let state = |text: &str| {
        let ours = text.contains(&begin);
        json!({"serverKey":key,"configured":if ours {json!(["codex"])} else {json!([])},"agents":[{
            "id":"codex","name":"Codex","detected":path.parent().is_some_and(|d| d.exists()),"manual":false,"reason":null,
            "installed": ours || has_table(text, &key)
        }]})
    };
    let write = |next: String| -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        if path.exists() {
            let _ = std::fs::copy(&path, path.with_extension("toml.cosmonic.bak"));
        }
        let tmp = path.with_extension("toml.cosmonic.tmp");
        std::fs::write(&tmp, next)
            .and_then(|_| std::fs::rename(&tmp, &path))
            .map_err(|e| e.to_string())
    };
    let outcome = match action {
        "state" => {
            return Ok(CallToolResult::structured(
                json!({"ok":true,"state":state(&read())}),
            ))
        }
        "configure" => {
            let url = url
                .filter(|u| {
                    (u.starts_with("http://") || u.starts_with("https://"))
                        && u.len() <= 2048
                        && !u
                            .chars()
                            .any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\\')
                })
                .ok_or_else(|| {
                    ErrorData::invalid_params(
                        "`url` must be the workload's http(s) MCP endpoint",
                        None,
                    )
                })?;
            let text = read();
            if !text.contains(&begin) && has_table(&text, &key) {
                Err(format!("{} already has a [mcp_servers.{key}] table that Cosmonic did not write; leaving it alone", path.display()))
            } else {
                let block = format!("{begin}\n[mcp_servers.{key}]\nurl = {}\n{end}", json!(url));
                let base = strip_block(&text, &key);
                let base = base.trim_end();
                write(if base.is_empty() {
                    format!("{block}\n")
                } else {
                    format!("{base}\n\n{block}\n")
                })
            }
        }
        "unconfigure" => {
            let text = read();
            if text.contains(&begin) {
                write(strip_block(&text, &key))
            } else {
                Ok(())
            }
        }
        _ => return Err(ErrorData::invalid_params("unknown action", None)),
    };
    let after = read();
    Ok(match outcome {
        Ok(()) => CallToolResult::structured(
            json!({"ok":true,"results":[{"id":"codex","ok":true,"error":null}],"state":state(&after)}),
        ),
        Err(error) => CallToolResult::structured(
            json!({"ok":false,"results":[{"id":"codex","ok":false,"error":error}],"state":state(&after)}),
        ),
    })
}

fn params<T: serde::de::DeserializeOwned>() -> Parameters<T> {
    Parameters(serde_json::from_value(json!({})).expect("default read params"))
}
fn unpack(label: &str, result: CallToolResult, errors: &mut Vec<String>) -> Value {
    let value = result.structured_content.unwrap_or(Value::Null);
    if result.is_error == Some(true) || value["status"] == "error" {
        errors.push(format!("{label}: {}", value["errors"]));
        Value::Null
    } else {
        value["result"].clone()
    }
}
pub async fn open(
    server: &CosmonicMcp,
    args: Option<Map<String, Value>>,
) -> Result<CallToolResult, ErrorData> {
    if args.is_some_and(|a| !a.is_empty()) {
        return Err(ErrorData::invalid_params(
            "This tool accepts empty arguments",
            None,
        ));
    }
    let (host, workloads, projects, templates) = tokio::join!(
        server.cosmonic_host_status(),
        server.cosmonic_workload_list(Parameters(
            serde_json::from_value(json!({"verbose":true})).unwrap()
        )),
        server.cosmonic_project_list(params::<tools::ListProjectsParams>()),
        server.cosmonic_template_list(params::<tools::ListTemplatesParams>()),
    );
    let mut errors = Vec::new();
    let host = unpack("Host", host, &mut errors);
    let workloads = unpack("Workloads", workloads, &mut errors);
    let workloads = tools::summarize_workloads(workloads);
    let projects = unpack("Projects", projects, &mut errors);
    let templates = unpack("Templates", templates, &mut errors);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    Ok(CallToolResult::structured(json!({"snapshot":{
        "host":host,"workloads":workloads,"projects":projects,"templates":templates,
        "observedAt":now,"errors":errors
    }})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opener_and_resource_have_ui_metadata() {
        let t = serde_json::to_value(&tools()[0]).unwrap();
        assert_eq!(t["_meta"]["openai/ui"]["entrypoints"][0]["type"], "global");
        assert_eq!(t["inputSchema"]["additionalProperties"], false);
        let r = serde_json::to_value(resource(uri())).unwrap();
        assert_eq!(r["mimeType"], MIME);
        let html = r["text"].as_str().unwrap();
        assert!(
            !html.contains("<script type=\"module\" crossorigin src="),
            "the app must be inlined"
        );
        assert!(html.contains("cosmonic_desktop_daemon_get"));
    }
    #[test]
    fn workspace_uri_is_versioned_by_its_html() {
        let u = uri();
        let digest = u
            .strip_prefix(BASE_URI)
            .and_then(|d| d.strip_prefix('/'))
            .unwrap();
        assert_eq!(digest.len(), 12);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        // Every tool points at this build's URI; the old one is still served.
        for t in tools().iter().filter(|t| t.meta.is_some()) {
            let m = serde_json::to_value(&t.meta).unwrap();
            assert_eq!(m["ui"]["resourceUri"], u, "{}", t.name);
        }
        assert!(is_uri(u) && is_uri(BASE_URI) && !is_uri("ui://cosmonic/other"));
        assert_eq!(
            serde_json::to_value(resource(BASE_URI)).unwrap()["uri"],
            BASE_URI
        );
    }
    #[test]
    fn app_tools_are_hidden_from_the_model() {
        for t in tools()
            .iter()
            .filter(|t| APP_TOOLS.contains(&t.name.as_ref()))
        {
            let v = serde_json::to_value(t).unwrap();
            assert_eq!(v["_meta"]["ui"]["visibility"], json!(["app"]), "{}", t.name);
        }
    }
    #[test]
    fn read_allowlist() {
        for ok in [
            "/v1/host",
            "/v1/workloads",
            "/v1/oci",
            "/v1/inspect?image=ghcr.io%2Fa%2Fb%3A1",
            "/v1/logs?limit=500&since=12",
            "/v1/system/workloads",
            "/v1/secrets/refs",
            "/v1/workloads/default/x/revisions",
            "/v1/projects/p/dev/logs",
        ] {
            assert!(readable(ok), "{ok}");
        }
        for bad in [
            "/v1/events",
            "/v1/secrets",
            "/v1/secrets/x/value",
            "/v1/../v1/host",
            "/v2/host",
            "/v1/host#x",
            "/v1/host\r\nX: y",
            "/v1//host",
            "/v1/shutdown",
        ] {
            assert!(!readable(bad), "{bad}");
        }
    }
    #[test]
    fn write_allowlist() {
        assert!(writable("POST", "/v1/workloads/default/web/stop"));
        assert!(writable("POST", "/v1/workloads/default/web/restart"));
        assert!(writable("DELETE", "/v1/workloads/default/web"));
        assert!(writable("DELETE", "/v1/oci/sha256%3Aabc/lock"));
        assert!(writable("POST", "/v1/oci/prune"));
        assert!(writable("POST", "/v1/workloads"));
        assert!(writable(
            "POST",
            "/v1/workloads/default/web/revisions/3/activate"
        ));
        assert!(writable("POST", "/v1/secrets/refs"));
        assert!(writable("PUT", "/v1/nats"));
        assert!(writable("POST", "/v1/projects/new"));
        assert!(!writable("POST", "/v1/registries"));
        assert!(!writable("POST", "/v1/registries/test"));
        assert!(carries_secret(
            &json!({"name":"k","uri":"keychain://k","value":"hunter2"})
        ));
        assert!(!carries_secret(
            &json!({"name":"k","uri":"op://vault/item/field"})
        ));
        assert!(carries_secret(
            &json!({"registry":"ghcr.io","username":"u","password":"p"})
        ));
        assert!(!writable("POST", "/v1/shutdown"));
        assert!(!writable("DELETE", "/v1/workloads/default"));
        assert!(!writable("POST", "/v1/workloads/default/web/stop?x=1"));
        assert!(!writable("GET", "/v1/workloads/default/web"));
    }
    #[test]
    fn codex_block_round_trips() {
        let (begin, end) = markers("weather");
        let text = "model = \"x\"\n\n[mcp_servers.other]\nurl = \"y\"\n";
        let block = format!(
            "{begin}\n[mcp_servers.weather]\nurl = \"http://weather.localhost:8200/\"\n{end}"
        );
        let with = format!("{}\n\n{block}\n", text.trim_end());
        assert!(has_table(&with, "weather"));
        assert_eq!(
            strip_block(&with, "weather"),
            "model = \"x\"\n\n[mcp_servers.other]\nurl = \"y\"\n\n"
        );
        assert_eq!(server_key("default", "weather"), "weather");
        assert_eq!(server_key("team", "weather"), "team-weather");
    }
    #[test]
    fn open_url_is_loopback_only() {
        for bad in [
            "https://example.com/",
            "http://evil.com@ae.localhost:8200/",
            "file:///etc/passwd",
            "http://ae.localhost.cosmonic.sh.evil.com/",
            "javascript:alert(1)",
        ] {
            assert!(open_url(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn workload_keys_are_safe_path_segments() {
        assert!(workload_key("default/sec-edgar-mcp").is_ok());
        assert!(workload_key("default/../x").is_err());
        assert!(workload_key("default").is_err());
        assert!(workload_key("default/a b").is_err());
    }
}
