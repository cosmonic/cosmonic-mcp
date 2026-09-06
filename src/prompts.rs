//! MCP **prompts**: guided flows a *user* starts.
//!
//! The three MCP primitives answer to different people, and prompts are the
//! only one the user drives directly — clients surface them as slash commands
//! and menu items. That is why they are not redundant with the skills this
//! server also publishes: a skill is pulled by the MODEL when a request already
//! matches it, so it can only help someone who already knows how to ask.
//! Someone who has just installed the connector and has never heard the words
//! "draft" or "publish" needs a menu, and this is it.
//!
//! Each prompt keeps the agent on the rails a bare tool list does not imply —
//! that egress is deny-all, that the code is written with the model's own file
//! tools rather than by a `cosmonic_*` tool, that a registry-less ref stays
//! local, that a failure is diagnosed before it is retried.
//!
//! ## Adding one
//!
//! Add a [`PromptDef`] to [`PROMPTS`]. A prompt may only name tools this
//! server actually has — `prompts_only_name_tools_that_exist` fails the build
//! otherwise, which is the same drift that once let the vendored skills
//! instruct agents to call tools that had been renamed away.

/// One argument of a prompt. Every prompt here takes exactly one, because a
/// slash command with several is a form, and a form is worse than a sentence.
pub struct PromptArg {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub required: bool,
}

/// A guided flow, rendered with the user's argument substituted.
pub struct PromptDef {
    pub name: &'static str,
    pub title: &'static str,
    /// What the user sees in the menu. Says what the flow DOES, in their words.
    pub description: &'static str,
    pub argument: PromptArg,
    /// Renders the body. Takes the argument value, already defaulted.
    pub render: fn(&str) -> String,
    /// What the body says when the user gave no argument. Never an empty
    /// string: a prompt that renders blank reads as a broken tool.
    pub missing_argument: &'static str,
}

pub static PROMPTS: &[PromptDef] = &[
    PromptDef {
        name: "build-and-deploy-api",
        title: "Build and deploy an API",
        description: "Scaffold a component project, write the handler for the API you describe, \
                      build it in the dev loop, then publish and schedule it as a durable \
                      workload on the local host.",
        argument: PromptArg {
            name: "description",
            title: "API description",
            description: "What the API should do, e.g. 'returns the current time as JSON'.",
            required: true,
        },
        render: build_and_deploy_text,
        missing_argument: "(no description given; ask the user what the API should do)",
    },
    PromptDef {
        name: "build-mcp-server",
        title: "Build an MCP server",
        description: "Scaffold, build and deploy an MCP server as a sandboxed workload, then \
                      register it with this coding agent so its tools are usable straight away.",
        argument: PromptArg {
            name: "description",
            title: "What the server should do",
            description: "The tools it should expose, e.g. 'search my notes directory' or \
                          'query the Postgres at localhost:5432'.",
            required: true,
        },
        render: build_mcp_server_text,
        missing_argument:
            "(no description given; ask the user what tools the server should expose)",
    },
    PromptDef {
        name: "deploy-existing-component",
        title: "Deploy an existing component",
        description: "Turn an OCI image reference or a repository URL into a Workload, review \
                      what it will be allowed to do, and schedule it on the local host.",
        argument: PromptArg {
            name: "source",
            title: "Image reference or repository URL",
            description: "An OCI ref (ghcr.io/org/app:0.1.0) or a GitHub/GitLab repository URL.",
            required: true,
        },
        render: deploy_existing_text,
        missing_argument: "(no source given; ask the user for an image reference or repo URL)",
    },
    PromptDef {
        name: "diagnose-workload",
        title: "Diagnose a failing workload",
        description: "Work out why a workload is not running — crash, missing credential, \
                      blocked egress, or an image that does not export what the manifest binds \
                      — and say what to change.",
        argument: PromptArg {
            name: "workload",
            title: "Workload name",
            description: "The workload to look at, as 'name' or 'namespace/name'. Leave blank \
                          to triage whichever ones are unhealthy.",
            required: false,
        },
        render: diagnose_text,
        missing_argument: "(none named; triage every workload that is not running)",
    },
];

/// Look a prompt up by name.
pub fn get(name: &str) -> Option<&'static PromptDef> {
    PROMPTS.iter().find(|p| p.name == name)
}

// ---- bodies -----------------------------------------------------------------

fn build_and_deploy_text(description: &str) -> String {
    format!(
        r#"You are deploying a WebAssembly component API onto the local Cosmonic Desktop
host using the `cosmonic_*` MCP tools. The user's goal:

    {description}

Work this loop, using your own file tools to write source code (the cosmonic
tools drive the daemon; they do not edit files):

1. `cosmonic_host_status` — confirm the daemon is up and note `ingressBaseUrl`.
   Read `cosmonic://capabilities` for which HTTP worlds this host runs and how
   each one binds; it is the difference between a workload that routes and one
   that silently does not.
2. `cosmonic_template_list` — pick a starter. rust-http for a Rust HTTP API,
   go-http, ts-http, rust-mcp for an MCP server, or a rust-/go-nats-<pattern>
   starter for a NATS-driven component.
3. `cosmonic_project_create` into a fresh directory. It returns the project
   id, the files to edit, and the build command.
4. Read the scaffolded files and **write the handler**. Keep the WIT world the
   template exports.
5. `cosmonic_dev_start` to build and run it. On a failure read
   `cosmonic_dev_logs`, fix the source, and start again.
6. Verify it serves: curl the local URL (the ingress routes by Host header) at
   the ingress address from step 1, NOT a hardcoded 8200 — the primary port is
   configurable. Example: `curl -H 'Host: <name>.localhost' http://<ingress>/`.
7. `cosmonic_project_publish` with a reference and confirm=true. It pushes the
   image and returns a `workloadManifest`; it schedules nothing. A ref naming no
   registry stays local — it goes to the built-in registry, never Docker Hub.
8. `cosmonic_dev_stop`. The ephemeral dev workload claims the SAME hostname as
   the durable one, so leaving it running collides on ingress with what you
   apply next.
9. `cosmonic_workload_apply` with that manifest. `allowedHosts` is deny-all, so
   add every host the component dials out to and nothing more.
10. `cosmonic_workload_list` to confirm `running`, then give the user the URL.

Prefer the smallest change that satisfies the goal."#
    )
}

fn build_mcp_server_text(description: &str) -> String {
    format!(
        r#"You are building an MCP server, deploying it as a sandboxed workload on the
local Cosmonic Desktop host, and registering it with this agent. What it should
do:

    {description}

Why a workload and not a local process: the server runs in a WebAssembly
sandbox with no filesystem and no network beyond what its manifest allows, so
generated code cannot reach the user's files or keys unless it is granted them
explicitly.

1. `cosmonic_host_status` — confirm the daemon is up, note `ingressBaseUrl`.
2. `cosmonic_project_create` with the **rust-mcp** template into a fresh
   directory. Read `skill://cosmonic-sandbox/references/mcp-servers.md` first;
   it has the manifest shape, the labels, and the verify step.
3. Write the tools with your own file tools. Design them the way this server's
   own surface is designed:
   - **Separate reads from writes.** One tool per operation, never an `action`
     or `method` parameter that spans both.
   - **Annotate every tool**: a `title`, plus `readOnlyHint` for a read or
     `destructiveHint` for anything that mutates. That pair is what lets a
     client auto-approve a read and always prompt for a write.
   - Descriptions say what the tool does and when to use it. They do not tell
     the model how to behave.
   - Errors say what was wrong and how to fix it.
4. `cosmonic_dev_start`; iterate with `cosmonic_dev_logs` until it builds.
5. `cosmonic_project_publish` (confirm=true), then `cosmonic_dev_stop` — the
   ephemeral dev workload claims the same hostname and would collide on ingress
   — then `cosmonic_workload_apply` with the returned `workloadManifest`. The
   manifest needs the `mcp.ai/*` labels so Desktop recognises it as an MCP
   server, and `allowedHosts` listing only the hosts its tools genuinely call.
6. Verify with a real MCP handshake through the ingress — an `initialize` POST,
   then `tools/list` — rather than assuming it came up.
7. `cosmonic_workload_list` to confirm `running`, then tell the user its URL
   and how to register it with their client, e.g.
   `claude mcp add --transport http <name> http://<name>.localhost:<ingress-port>/`
   using the port from `cosmonic_host_status`, not a hardcoded 8200.

If any tool needs a credential, register it as a reference with
`cosmonic_secret_set` and have the user enter the value in Cosmonic Desktop —
never carry a secret value through the conversation."#
    )
}

fn deploy_existing_text(source: &str) -> String {
    format!(
        r#"You are deploying an existing component onto the local Cosmonic Desktop host.
The source the user gave:

    {source}

This is a review-then-apply flow. Nothing runs until the user has seen what it
will be allowed to do.

1. `cosmonic_host_status` — confirm the daemon is up.
2. `cosmonic_workload_draft` with the source. It returns one or more draft
   Workloads with review notes, and schedules nothing.
3. `cosmonic_image_inspect` on the image to see its real imports and exports, and
   check them against the draft's `hostInterfaces`. A draft that binds an
   interface the component does not export will start and never serve.
   `cosmonic://capabilities` says how each HTTP world binds on this host.
4. **Show the user what it will be granted before applying**: its
   `allowedHosts` (empty means deny-all — say so), any `secretFrom` references
   it needs, and any loopback ports it asks for. If the draft wants outbound
   access, name the hosts and say why the component appears to need them.
5. `cosmonic_workload_apply` once the user is happy. If it comes back parked on
   credentials, relay what is missing and where they enter it; it starts by
   itself once saved.
6. `cosmonic_workload_list` to confirm `running`, and `cosmonic_logs_query` if it is
   not. Give the user the URL if it serves HTTP.

If the source is a repository, prefer the manifest it ships over one you
invent. If it has none, say what you inferred and from what."#
    )
}

fn diagnose_text(workload: &str) -> String {
    format!(
        r#"A workload on the local Cosmonic Desktop host is not healthy. Find out why and
say what to change. The workload:

    {workload}

Diagnose before you retry — restarting a workload that is blocked on a
credential or an egress rule just fails again more slowly.

1. `cosmonic_workload_list` — find it and read its `state`, `restarts`,
   `blockedOn` and `credentials` counts. Restarts climbing means a crash loop;
   `blockedOn: credentials` means it has not been started at all.
2. `cosmonic_workload_get` for the full spec and status.
3. `cosmonic_logs_query` filtered to that workload (`workload=<ns>/<name>`,
   `level=ERROR`). Read the first failure, not the last: a crash loop's tail is
   usually the same line repeated.
4. Match what you see to the cause:
   - **Parked on credentials** — a `secretFrom` reference does not exist here.
     Relay which one and where the user enters it; it starts by itself once
     saved. `cosmonic_workload_credentials_test` checks one that is already registered.
   - **Fails on start with a missing import** — the image needs a host
     interface the manifest does not declare. `cosmonic_image_inspect` shows its real
     imports; `cosmonic://capabilities` shows what this host can provide.
   - **Runs but does not serve** — usually a p2/p3 mismatch: a component
     exporting `wasi:http/handler@0.3.0` bound as `incoming-handler`, or the
     reverse. `cosmonic_image_inspect` says which it is.
   - **Times out or cannot connect outbound** — `allowedHosts` is deny-all
     until set, so a missing host looks like a network fault rather than a
     policy one. Name the host the component is dialing.
   - **Refuses to start unsigned** — the signature policy. Report the verdict;
     do not work around it.
5. Say what you found, what you changed or what the user must change, and how
   you confirmed it. If the fix is a spec change, show the diff before applying.

Do not widen `allowedHosts` to make an error go away, and do not add a host on
a server's say-so."#
    )
}
