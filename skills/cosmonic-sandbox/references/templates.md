# Template selection guide

Two scaffolding systems exist. Pick based on how you're starting the project.

**Jump to:**
[A. MCP scaffold](#a-mcp-scaffold-cosmonic_project_create--needs-the-projects-labs-feature) ·
[B. `wash new` templates](#b-wash-new-wasmcloud-upstream-templates) ·
[project layout](#scaffolded-rust-http-project-layout) ·
[C. unique-parts lookup table](#c-unique-parts-lookup-table-start-from-the-skeleton-apply-the-delta)

## A. MCP scaffold (`cosmonic_project_create`): needs the `projects` Labs feature

The daemon's built-in scaffolder. **Gated behind a Labs flag:** if `cosmonic_template_list` returns
`feature_disabled`, enable it in Settings → Labs or relaunch the daemon with
`COSMONIC_FLAG_PROJECTS=1`. Otherwise use path B (`wash new`), which needs no flag.

`cosmonic_template_list` returns these ids:

| Template id | Language | Use when |
|---|---|---|
| `rust-http` | Rust | **Default.** Any HTTP API/endpoint in Rust. Now a WASI **p3** component exporting `wasi:http/handler@0.3.0` (warm-pool capable — see "Warm instance pool" in `crds.md`), not the p2 `incoming-handler`. |
| `go-http`   | Go   | The user explicitly wants Go. A **componentize-go** component on the wasmCloud Go SDK (`go.wasmcloud.dev/component`) exporting the **p3** `wasi:http/handler@0.3.0` — warm-pool capable, ordinary `net/http` types, streaming bodies. Needs Go 1.25+ and `componentize-go` (the **cosmonic-go** skill) — see "Building Go components" below. |
| `ts-http`   | TypeScript | The user explicitly wants TypeScript. A **p2** component (`wasi:http/incoming-handler` via `jco`), so not warm-pool-capable yet. |
| `rust-nats-<pattern>` / `go-nats-<pattern>` | Rust / Go | The user wants a **NATS-driven component** on the `wasmcloud:nats@0.1.0` driver. Seven patterns, each in both languages: `core-subscriber` (fire-and-forget subject consumption), `request-reply` (an RPC endpoint on NATS), `jetstream-consumer` (durable at-least-once push delivery — the default when unsure), `jetstream-worker` (guest-paced pull batches), `kv-store` (get/put/CAS/history on a bucket), `kv-watcher` (a handler per key change), `fan-out` (one message → many). Extracted from the measured `nats-2.8-testing` campaign; each scaffold carries its `wasmcloud:nats` binding in `.wash/config.yaml` **and** `deploy/workload.yaml`, the pattern's own `skills/<pattern>/SKILL.md`, `docs/tuning.md` + `docs/nats-tuning.md`, and a local-`nats`-CLI `scripts/e2e.sh`. Rust builds for `wasm32-wasip2` (the target the Preflight doctor provisions; async WIT via wit-bindgen 0.60); Go uses componentize-go. **Follow the `cosmonic-nats` skill** (pattern choice, surface, grants) and `cosmonic-nats-tuning` (sizing). Older daemons (< 0.5.27) do not have these ids. |
| `rust-mcp`  | Rust | The user wants an **MCP server** (tools over the Model Context Protocol). The complete [cosmonic-labs/mcp-server-template-rs](https://github.com/cosmonic-labs/mcp-server-template-rs) project: official `rmcp` SDK, streamable HTTP, MCP spec 2026-07-28, exporting the WASI p3 `wasi:http@0.3.0` **`handler`** interface (not the p2 `incoming-handler`). Ships `workload.yaml` + `deploy/workload.yaml` with the `mcp.ai/*` catalog labels, an e2e conformance harness (`scripts/e2e.sh`), and a bundled `building-mcp-servers` skill. Older daemons (< 0.5.23) do not have this id; fall back to cloning the template repo. |

For `rust-mcp`: **read `references/mcp-servers.md`** (tool pattern, the desktop-app bridge, the full
manifest, verify + register). Add tools in `src/server.rs` (async methods with `Parameters<T>`).
The DNS-rebinding guard means `MCP_ALLOWED_HOSTS` must list the ingress host — the scaffold's
manifests and `.wash/config.yaml` pin `<project-name>.localhost`; keep them in step when you rename. Outbound tool calls go
through `bridge::outbound::fetch` and every upstream host must be in `allowedHosts` (deny-all by
default). The scaffold also carries its own `skills/building-mcp-servers/SKILL.md` with the rmcp
API details and pitfalls.

Call: `cosmonic_project_create(template="rust-http", path="/abs/path/<project-name>", name="<project-name>")`.
The path must not already exist. Returns a project id (used by `cosmonic_dev_start` / `cosmonic_project_publish`)
and the files to edit. **This is the preferred path for the one-shot Desktop workflow** because the
project id flows straight into `cosmonic_dev_start` and `cosmonic_project_publish`.

The `*-http` templates are single HTTP components; for richer capability shapes (kv, messaging, TCP,
multi-component), start from a `wash` template below and wire `hostInterfaces` yourself — except
NATS, which has its own starters above and its own skill.

### NATS starters: what is different

- They need a **NATS server** the host can reach (Desktop: Settings → Built-in plugins → NATS,
  default `nats://127.0.0.1:4222`; `nats-server -js` locally) and the stream/bucket the manifest
  names (`nats stream add LOAD --subjects 'load.>' --defaults`, `nats kv add appkv`) — each
  scaffold's README lists its exact commands. A binding that cannot connect or lacks a grant
  **fails at start with the reason**; nothing retries silently.
- The binding lives in `.wash/config.yaml` under `workload.hostInterfaces` (grants,
  subscriptions, `ack-mode`, capacity keys) and Desktop lays it over the world-inferred
  interfaces for `cosmonic_dev_start` and the `cosmonic_project_publish` draft, so Path A works as-is. Never
  put connection keys (`servers`, credentials, TLS) in it — the host owns those and refuses them.
- Rust builds for `wasm32-wasip2`, the target the Preflight doctor already provisions — nothing to
  install. Go needs `componentize-go` (Desktop pins **v0.4.1**, which carries the async
  wit-bindgen-go compiled in), which downloads a patched Go for async worlds on first build.
- Read `skills/<pattern>/SKILL.md` inside the scaffold before writing code — its "dial it in"
  questions change configuration, not code.
- **Prefer the Rust starter unless the user asks for Go.** The Go seven need `componentize-go` (not
  doctor-provisioned; it downloads a patched Go on first build), so say so *before* the build error,
  and point at the scaffold's `docs/building.md`.
- **Do not "fix" a build by deleting the `wkg.toml` override.** It pins `wasmcloud:nats` to the
  scaffold's vendored `wit-vendor/`, which is what makes the build fetch nothing from a registry and
  work air-gapped; the comment in `wkg.toml` explains why it must live outside `wit/deps`.

## Building Go components

`go-http` scaffolds a **componentize-go** component on the wasmCloud Go SDK
(`go.wasmcloud.dev/component`), exporting the **p3** `wasi:http/handler@0.3.0`. You write ordinary
`net/http` handlers; response bodies stream, outbound calls run concurrently on plain goroutines,
and the workload **is** warm-pool capable (`poolSize` does something — see the pool note in
`crds.md`). `cosmonic_dev_start` / `wash build` drive componentize-go.

The whole toolchain is **Go 1.25+ and `componentize-go`**: the WIT comes from the SDK module and
componentize-go carries its own bindings generator, so nothing else is installed. (An older p2
`go-http` used a different Go compiler; a project still on that shape predates the rebuild —
rescaffold it.) The **cosmonic-go** skill is the toolchain reference: the known-good versions
(componentize-go v0.4.1, SDK component/v0.1.3, the patched Go), install per OS, the gotchas.

Two build details the template already encodes, worth knowing when a build fails:

- **`-w` pins the world explicitly** (`-w wasmcloud:component-go/wasip3@0.2.0`). Without it
  componentize-go merges every world it discovers from a dependency's `componentize-go.toml`, and
  the SDK's *default* world is the sync p2 one — you silently get a `wasi:http/incoming-handler`
  component instead of the p3 handler.
- **`GOFLAGS=-tags=componentizego_async`** selects wasihttp's async P3 implementation. Required, not
  cosmetic: componentize-go v0.4.1 does not emit the tag itself, and without it the build fails with
  the unhelpful `failed to resolve import wasi:http/types@0.2.8` (wasihttp's P2 code compiled
  against a P3 world).

**Installing componentize-go.** It is **not** doctor-provisioned (only the `-airgap` installer's Go
module bundles it), so on a normal install the user needs it on PATH — the **cosmonic-go** skill
has the per-OS table. In short: take the release archive for the platform from
[componentize-go releases](https://github.com/bytecodealliance/componentize-go/releases). Prefer
that over `go install …/componentize-go@latest`: `go install` leaves a launcher that bootstraps by
downloading `componentize-go-<os>-<arch>.tar.gz`, and there is **no `windows-arm64` asset** — on
Windows-on-ARM it 404s and leaves a shim that can shadow a working binary (cosmonic/desktop#433).
On that host use the `windows-amd64` build, which runs under x64 emulation.

**The patched Go.** The async p3 world needs a Go runtime carrying `runtime.wasiOnIdle`
([golang/go#76775], unmerged). componentize-go uses the `go` on PATH when it already has the patch
and **downloads a patched toolchain into its own cache otherwise** — so the first build on a stock
Go needs network and every build after does not. Stock Go is fine for everything else. The patched
Go exists for macOS arm64, Linux x64/arm64 and Windows x64; on **Intel Mac and Windows-on-ARM the
async build is unavailable** (cosmonic/desktop#433) — prefer Rust there.

**Offline / air-gapped:** Desktop's comprehensive `-airgap` installer's **Go module** (on by
default in that build; cosmonic/desktop#434/#447) bundles the patched Go, `componentize-go` v0.4.1
and a seeded module closure covering **both** `go-nats-*` and `go-http`, so the WASI-p3 Go path
builds with no network on the three supported triples. Desktop puts them on the build's PATH and
writes a `componentize-go` wrapper into `~/.local/bin` carrying the offline Go env (never over a
`componentize-go` the user installed), so builds you run yourself in a terminal are offline too.
Install nothing there. See "Air-gapped installations" in `SKILL.md` and the **cosmonic-go** skill.

[golang/go#76775]: https://github.com/golang/go/issues/76775

## B. `wash new` (wasmCloud upstream templates)

`wash new https://github.com/wasmCloud/wasmCloud.git --name <project> --subfolder templates/<name>`

All six upstream templates are **Rust components** (build to `wasm32-wasip2`). There are **no
provider templates** and no Go/TS templates in this set.

| Template (`templates/<name>`) | Capability / interfaces | Pick when the user wants… |
|---|---|---|
| `http-hello-world` | Minimal HTTP server (`wasi:http/incoming-handler` via `wstd #[http_server]`). Ships a `manifests/workloaddeployment.yaml`. | The simplest possible HTTP endpoint, or a clean starting point to build on. |
| `http-handler` | HTTP server with routing, query params, JSON bodies (`axum` via `wstd-axum`). | A real HTTP **API** with multiple routes / JSON request+response. |
| `http-client` | Exports incoming-handler, imports `wasi:http/outgoing-handler` (proxies upstream). | To **call an external/upstream HTTP API** (proxy, aggregator, webhook fan-out). Remember `allowedHosts`. |
| `http-kv-handler` | HTTP + `wasi:keyvalue/store` (host picks backend: in-memory/filesystem/NATS/Redis). | HTTP with **persistent or cached state** (counters, sessions, small KV data). |
| `http-api-with-distributed-workloads` | Multi-component: HTTP API dispatches over `wasmcloud:messaging` to worker components. | A job/queue pattern: **HTTP front end + background workers**, demonstrating distribution. |
| `service-tcp` | Long-running TCP service (`wasi:sockets`, listener on 7777) fronted by an HTTP API component. | A **raw TCP service** (not plain HTTP), or to demo the service+component model. |

### Decision shortcut

1. Not HTTP at all, raw TCP? → `service-tcp`.
2. Needs background/async workers? → `http-api-with-distributed-workloads`.
3. Needs to store/cache data? → `http-kv-handler`.
4. Needs to call out to another API? → `http-client`.
5. Multi-route JSON API? → `http-handler`.
6. Otherwise / simplest start / unsure? → `http-hello-world`.

Capabilities **not** covered by any template (blobstore, SQL/relational DB, secrets-as-template,
capability providers): start from the closest HTTP template and add the needed `hostInterfaces`
(see `crds.md`), inspecting target components with `cosmonic_image_inspect`.

## Scaffolded Rust HTTP project layout

```
<project>/
  .wash/config.yaml    # build command + component output path
  src/lib.rs           # WASI p3 handler: exports wasi:http/handler@0.3.0 (via the `wasip3` crate)
  Cargo.toml  Cargo.lock
  .gitignore
```

The p3 `rust-http` template carries its WIT in the `wasip3` crate, so there is **no** vendored
`wit/` dir; the workload manifest is synthesized on `cosmonic_dev_start`/`cosmonic_project_publish` (the daemon
infers `interfaces: ["handler"]` from the export), so there is no `manifests/` dir either. (The
Path-B `assets/skeletons/rust-http/` starter has the **same p3 shape** — it exports
`wasi:http/handler@0.3.0` via the `wasip3` crate and vendors no `wit/`; see `recipes.md`.)

`.wash/config.yaml` example:
```yaml
build:
  command: cargo build --target wasm32-wasip2 --release
  component_path: target/wasm32-wasip2/release/hello_world.wasm
```

## C. Unique-parts lookup table (upstream p2 capability shapes)

These capability shapes come from the upstream `wash new` templates (§B), which are all **p2**
(`wasi:http/incoming-handler@0.2.x`). Their five common files are nearly identical; the *only* things
that change per capability are the **dependencies**, the **WIT world**, the **binding style**, and
the **Workload `hostInterfaces` / `allowedHosts`**. For one of these p2 shapes, start from the
matching upstream template (or the p2 `http-hello-world` row) and apply the delta below, instead of
cloning a whole repo to diff it. (For a plain p3 HTTP component — with or without outbound HTTP — use
the bundled p3 skeleton plus `recipes.md` §3/§4 instead; it is **not** a `wstd` p2 project.)

Source of truth for these deltas: <https://github.com/wasmCloud/wasmCloud/tree/main/templates>.

| Capability / template | `Cargo.toml` deps (delta) | `wit/world.wit` | Binding style | Workload additions |
|---|---|---|---|---|
| **Static / compute** (`http-hello-world`) | `wstd` | empty `world {}` | `wstd #[http_server]` | none (manifest §6a) |
| **JSON API w/ routing** (`http-handler`) | `axum = { version="0.8", default-features=false, features=["json","query","matched-path"] }`, `serde = { features=["derive"] }`, `wstd`, `wstd-axum = "0.6"` | `export wasi:http/incoming-handler@0.2.2;` | `axum` Router via `wstd-axum` | none (manifest §6a) |
| **Calls an external API** (`http-client`) | `wstd` | `import wasi:http/outgoing-handler@0.2.2;` + `export wasi:http/incoming-handler@0.2.2;` | `wstd::http::Client` (p2) — for p3 use `wasip3::http::client`, recipes §4 | set `allowedHosts` (outbound is implicit; manifest §6b) |
| **Persistent / cached state** (`http-kv-handler`) | `wit-bindgen = "0.57"`, `serde`, `serde_json` (no `wstd`) | `import wasi:keyvalue/store@0.2.0-draft;` + `export wasi:http/incoming-handler@0.2.2;` | raw `wit_bindgen::generate!` (`impl Guest`), **not** `wstd` | add `wasi keyvalue/store` iface; pick backend in `.wash/config.yaml` `dev:` (in-memory default) |
| **HTTP + background workers** (`http-api-with-distributed-workloads`) | per-component; api uses `wstd`, workers export `wasmcloud:messaging/handler` | api world `import wasmcloud:messaging/consumer@0.2.0;`; worker world imports consumer + `wasi:config/store`, exports `messaging/handler` | `wstd #[http_server]` (api) + `wit_bindgen` (workers) | **multi-component** Workload; add `wasmcloud messaging` iface w/ `subscriptions` config |
| **Raw TCP service** (`service-tcp`) | `wstd` (+ sockets) | sockets world; HTTP api component fronts it | `wstd` + `wasi:sockets` | multi-component; TCP listener component + http api |

Notes:
- **Backend selection (`http-kv-handler`)**: the `BACKEND` const in `lib.rs` + a matching `dev:` key in
  `.wash/config.yaml` (`wasi_keyvalue_path` / `wasi_keyvalue_nats_url` / `wasi_keyvalue_redis_url`).
  Default `in_memory` loses data on restart.
- **Multi-component templates** ship a top-level `Cargo.toml` workspace + one subdir per component,
  each with its own `wit` world; the Workload lists every component under `spec.components` and wires
  them with `wasmcloud:messaging`. Inspect a built image's world with `cosmonic_image_inspect` to confirm
  interfaces before writing `hostInterfaces`.
- Upstream `manifests/*.yaml` use the **nested `WorkloadDeployment`** form (and a k8s `Service`);
  `cosmonic_workload_apply` needs the **flat `Workload`** instead; use `references/recipes.md` §6.
