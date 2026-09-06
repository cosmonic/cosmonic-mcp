# MCP servers on Cosmonic Desktop

Read this whenever the user asks for an **MCP server** — for a desktop app (Illustrator,
Photoshop, After Effects, Blender), a SaaS or public API (Stripe, GitHub, SEC EDGAR), a database,
or a local data source — whether or not they say "Cosmonic". The deliverable is a **sandboxed
WebAssembly component** serving MCP over **streamable HTTP** on the Desktop ingress, registered
with the user's MCP client. It is never a host-native npm `@modelcontextprotocol/sdk` stdio
process, a Python server, or an `osascript` shim.

**Jump to:** [pick the shape](#1-pick-the-shape) · [scaffold](#2-scaffold-rust-mcp) ·
[tools](#3-write-tools-srcserverrs) · [desktop-app bridge](#4-shape-c-a-desktop-application-bridge) ·
[Workload](#5-the-workload-manifest) · [verify + register](#6-verify-then-register-the-client) ·
[checklist](#7-checklist)

## 1. Pick the shape

| The server… | Shape | Extra capability | Worked example (cosmonic-labs/mcp-examples) |
|---|---|---|---|
| Computes from its inputs (math, codegen, conversion, timecode) | **A. pure compute** | none; `allowedHosts: []` | `premiere-mcp` |
| Calls an upstream HTTP API | **B. outbound** | `allowedHosts: [<api host>]`; a key via `secretFrom` | `sec-edgar-mcp` (no key), `fred-mcp` (key) |
| Drives a **desktop application** that has no network API | **C. bridge** — the app polls the component | `wasi:keyvalue` store for the command queue | `after-effects-mcp` |
| Talks to a local model | B with `allowedHostLoopbackPorts` | see `local-ai.md` | — |

A server can mix shapes (After Effects ships 6 pure-compute helpers beside 30 bridge tools, so
it still answers with the app closed). Decide the shape before writing a tool.

## 2. Scaffold `rust-mcp`

```
cosmonic_project_create(template="rust-mcp", path="<abs>/<NAME>", name="<NAME>")
```

The scaffold is the complete [mcp-server-template-rs](https://github.com/cosmonic-labs/mcp-server-template-rs)
project: official `rmcp` 3.x SDK, MCP spec **2026-07-28** (stateless streamable HTTP, `POST /`
only, SSE-framed responses), exporting the WASI p3 `wasi:http/handler@0.3.0`. It ships:

```
<NAME>/
  src/server.rs        # ← your tools go here (example echo/add/http_get tools to replace)
  src/lib.rs           # transport: request lock, body limits, MCP_ALLOWED_HOSTS guard
  src/bridge.rs        # tokio ↔ component-model driver + bridge::outbound::fetch
  src/telemetry.rs
  workload.yaml        # local loop: host <NAME>.localhost, image on the built-in registry
  deploy/workload.yaml # published image: host <NAME>.localhost
  .wash/config.yaml    # build cmd + the mcp.ai/* labels Desktop uses to catalog MCP servers
  scripts/e2e.sh       # conformance harness (runs under wasmtime; no Desktop needed)
  skills/building-mcp-servers/SKILL.md   # the template's own skill: rmcp API, pitfalls — read it
  docs/auth.md  README.md  Cargo.toml  Cargo.lock  .cargo/config.toml
```

If `cosmonic_template_list` reports `feature_disabled` (older daemon or Labs flag off), copy the
template repo instead (`cp -R` its `.cargo .wash src scripts deploy Cargo.toml Cargo.lock
workload.yaml .gitignore`) and rename `mcp-server-template` / `mcp-server` throughout. Builder may
already have scaffolded it for you (SKILL.md step 0).

**Three names must agree** after renaming: `Cargo.toml` `[package] name` (kebab-case),
`.wash/config.yaml` `component_path` (`target/wasm32-wasip2/release/<name_with_underscores>.wasm`),
and `metadata.name` + `config.host` + `MCP_ALLOWED_HOSTS` in both manifests.

## 3. Write tools (`src/server.rs`)

One params struct per tool with a doc comment on every field (they become the client-visible JSON
schema), one `#[tool]` method. Replace the template's example tools; keep the macros.

```rust
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, ErrorData};
use rmcp::tool;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HexToRgbParams {
    /// Hex color like "#5eead4" or "5eead4".
    pub hex: String,
}

#[tool_router]
impl MyServer {
    #[tool(description = "Convert a hex color to 0..1 RGB floats and 0..255 ints")]
    #[tracing::instrument(name = "tool.hex_to_rgb", skip(self))]
    async fn hex_to_rgb(
        &self,
        Parameters(p): Parameters<HexToRgbParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let Some(rgb) = parse_hex(&p.hex) else {
            // The tool ran and failed: the caller sees this message.
            return Ok(CallToolResult::error(vec![ContentBlock::text("not a hex color")]));
        };
        // Structured output: fills structuredContent and a text fallback.
        Ok(CallToolResult::structured(serde_json::json!({ "r": rgb.0, "g": rgb.1, "b": rgb.2 })))
    }
}
```

Rules the template's skill spells out (follow them; they are the difference between a tool that
works under a real client and one that traps):

- `Err(ErrorData::invalid_params(..))` **only** for a request the server cannot parse or route;
  `Ok(CallToolResult::error(..))` for "ran and failed". Prefer `structured(..)` whenever the output
  has shape.
- **No per-session state on the server struct.** Instances are ephemeral and requests may land on
  different instances; durable state goes in a host capability (`wasi:keyvalue`) or upstream.
- **Never panic.** `panic=abort` kills the instance: no `unwrap`/`expect`/`[i]`, no
  `String::truncate` at a byte offset, `checked_*` on client-supplied integers, integer `/` and `%`
  only after validating the divisor.
- **Outbound HTTP goes through `crate::bridge::outbound::fetch`** (deadline + size cap), never a
  direct WASI future from tool code; every upstream host must be in `allowedHosts`. Keep
  `wit-bindgen`'s `inter-task-wakeup` feature (already in the template's `Cargo.toml`) or
  concurrent outbound calls trap.
- Keys come from env (`std::env::var`), injected via `secretFrom` + `cosmonic_secret_set`; a
  missing key returns a distinct, actionable tool error.
- Update `get_info()`'s `with_instructions(..)` to describe the server and how to use its tools.

Build gate: `cargo build --release` then confirm the export:
`wasm-tools component wit target/wasm32-wasip2/release/<name>.wasm | grep 'export wasi:http/handler@0.3.0'`.
`cargo test` does not apply (wasm target); `scripts/e2e.sh` is the test entry point.

## 4. Shape C: a desktop-application bridge

A sandboxed component cannot open the app, run AppleScript, or reach an app-local socket — and it
should not. **Invert the call:** a small script *inside* the app polls the component for work.

```
MCP client ──tools/call──▶ component ──queue──▶ wasi:keyvalue
                               ▲                     │
                               └──result── panel ◀───poll──┘   (runs inside the app)
```

Lifted from `after-effects-mcp` (the reference implementation; copy it rather than re-deriving):

**Component side**

- Import `wasi:keyvalue/store@0.2.0-draft` via a second `wit_bindgen::generate!` world (add the
  `macros` feature to `wit-bindgen`, vendor `wit/world.wit` + `wit/deps/wasi-keyvalue-0.2.0-draft/`).
  Bucket `in_memory` (override `MCP_BRIDGE_BUCKET`); keys `<prefix>:command|result|seq|last-poll-ms|
  last-stale-poll-ms|client` with `<prefix>` from `MCP_BRIDGE_KEY_PREFIX` (the default bucket is
  host-wide; two bridges with one prefix steal each other's commands).
- A one-slot queue: `queue_command` writes `{id, command, args, status:"pending", queuedAtMs}`;
  the app's poll flips it to `dispatched`; the app's result POST stores `result` with `_commandId`
  and marks the command `completed`.
- Two routes served **before** the MCP request lock in `src/lib.rs` (a poll queued behind the tool
  call that is waiting for it deadlocks):
  `GET /bridge/command?v=2&client=<id>` → the pending command or `{"command":null}`;
  `POST /bridge/result?id=<n>` (JSON body) → `{"status":"ok"}`. Highest `client` id wins, so a
  reloaded panel supersedes the old one. Add `GET /healthz`, and serve the panel script itself at
  `GET /bridge/panel.jsx` (`include_str!`) so the installed copy can never drift.
- A live tool is `dispatch("<commandName>", &params).await`: queue, then `wait_for_result(id, 12_000)`
  polling the store every 200 ms via `crate::bridge::sleep` (never `tokio::time`). Return three
  distinct non-results: never connected (tool error with install steps), went quiet (tool error),
  timed out but maybe still working (`status: "pending"`, not an error) plus a `get_results` tool.
  Ship `bridge_status` so the client can ask what is wrong.
- `poolSize: 4` so a tool call blocked on a result never starves the app's polls.

**App side** (whatever scripting the app offers: ExtendScript/ScriptUI for Adobe apps, UXP, a
VS Code extension, a Blender add-on, AppleScript `do shell script curl`): every ~2 s,
`GET /bridge/command`, execute the command against the app's API, `POST /bridge/result` (retry 3×).
Send `Host: <NAME>.localhost` to `127.0.0.1:8200` — the ingress
routes by Host header. Make the Host value editable in the panel. Batch: expose a `run_batch`
tool so a whole scene is one poll cycle.

**Test without the app**: `scripts/e2e.sh` composes a `wasi:keyvalue` stub with `wac plug` and
plays the panel with three curls — poll, claim, answer — while the tool call runs in the
background. Copy that harness.

## 5. The Workload manifest

Apply with `cosmonic_workload_apply` (flat `Workload`; JSON or YAML). Use the digest-pinned image
`cosmonic_project_publish` returned, or the published ref.

```yaml
apiVersion: runtime.wasmcloud.dev/v1alpha1
kind: Workload
metadata:
  name: <NAME>
  namespace: default
  labels:                                  # Desktop catalogs mcp.ai/* workloads as MCP servers
    app.kubernetes.io/name: "<NAME>"
    app.kubernetes.io/version: "0.1.0"
    "mcp.ai/auth-type": none               # none | oauth
    mcp.ai/domain: "<subject-area>"        # e.g. adobe-illustrator, sec-edgar
    "mcp.ai/function-type": tools
    "mcp.ai/spec-version": "2026-07-28"
    mcp.ai/statefulness: stateless
    mcp.ai/transport: "streamable-http"
  annotations:
    desktop.cosmonic.com/source: mcp
    desktop.cosmonic.com/unsafe-allow-unsigned: "true"   # local unsigned image; dev only
spec:
  hostInterfaces:
    - namespace: wasi
      package: http
      interfaces: ["handler"]              # p3; never incoming-handler for this template
      config:
        host: "<NAME>.localhost"
    # Shape C only — the bridge queue; the component will not instantiate without it:
    # - namespace: wasi
    #   package: keyvalue
    #   interfaces: ["store"]
  components:
    - name: <NAME>
      image: oci.localhost:8200/apps/<NAME>@sha256:…    # publish's digest-pinned ref
      poolSize: 1                          # 4 for shape C
      maxInvocations: 0
      localResources:
        environment:
          config:
            RUST_LOG: info
            # DNS-rebinding guard: must list every host the ingress serves this workload as.
            MCP_ALLOWED_HOSTS: "<NAME>.localhost"
            # Shape C: MCP_BRIDGE_KEY_PREFIX: "<NAME>"
          # secretFrom:                    # shape B with a key (register via cosmonic_secret_set)
          #   - name: <NAME>-api-key       #   uri keychain://cosmonic/<NAME>-api-key, env API_KEY
        allowedHosts: []                   # shape B: ["api.example.com"]; C and A: deny-all
```

`MCP_ALLOWED_HOSTS` unset means rmcp accepts only `localhost`/`127.0.0.1`, so every request through
the ingress answers **403** — the most common "it deployed but nothing works" cause. The scaffold's
`workload.yaml` and `deploy/workload.yaml` both pin `<NAME>.localhost`; if you rename the project,
change `metadata.name`, `config.host`, and `MCP_ALLOWED_HOSTS` together, and report the URL from the
`config.host` you actually applied (SKILL.md, the hostname rule).

## 6. Verify, then register the client

Initialize through the ingress (the Host-header form verifies routing independent of DNS):

```bash
curl -s -X POST -H 'Host: <NAME>.localhost' http://127.0.0.1:8200/ \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2026-07-28' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
  | grep -o '"protocolVersion":"[^"]*"'          # expect "2026-07-28"
```

List tools (post-initialize requests carry `Mcp-Method` and a `_meta` block; real clients add
these themselves):

```bash
curl -s -X POST -H 'Host: <NAME>.localhost' http://127.0.0.1:8200/ \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2026-07-28' -H 'Mcp-Method: tools/list' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}' \
  | grep -o '"name":"[^"]*"' | head            # one line per tool
```

Call one tool the same way with `-H 'Mcp-Method: tools/call' -H 'Mcp-Name: <tool>'` and
`"params":{"name":"<tool>","arguments":{…},"_meta":{…}}`. Responses are SSE (`data:` lines).

Then hand the user the registration for their client (the server is stateless; any streamable-HTTP
client works):

```bash
claude mcp add --transport http <NAME> http://<NAME>.localhost:8200/
# Windows before 10 1709, or any resolver that does not special-case .localhost:
claude mcp add --transport http <NAME> --header 'Host: <NAME>.localhost' http://127.0.0.1:8200/
```

Claude Desktop (`claude_desktop_config.json`): `{"mcpServers":{"<NAME>":{"type":"http","url":"http://<NAME>.localhost:8200/"}}}`.
Cosmonic Desktop also detects the `mcp.ai/*` labels and can register the server into every
detected coding agent from its UI.

## 7. Checklist

- [ ] Shape chosen (A/B/C); for C the app-side poller is part of the deliverable.
- [ ] `rust-mcp` scaffolded (or Builder's pre-scaffold reused); three names agree.
- [ ] Tools replaced; params documented; errors follow the invalid-params vs tool-error rule; no panics.
- [ ] `allowedHosts` lists exactly the upstream hosts; keys via `secretFrom`.
- [ ] Workload carries `mcp.ai/*` labels, `desktop.cosmonic.com/source: mcp`, `interfaces: ["handler"]`,
      `MCP_ALLOWED_HOSTS` with both ingress names (+ `wasi:keyvalue` and `poolSize: 4` for C).
- [ ] `initialize` returns `2026-07-28` through the ingress; `tools/list` shows your tools.
- [ ] User given the URL and the `claude mcp add --transport http …` line.
