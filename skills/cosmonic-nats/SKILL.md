---
name: cosmonic-nats
description: Build a NATS-driven WebAssembly component for Cosmonic Desktop on the wasmcloud:nats@0.1.0 driver. Pick one of the seven golden templates (core subscriber, request/reply, JetStream consumer, JetStream pull worker, KV store, KV watcher, fan-out; Rust or Go), write the handler against the async WIT surface, declare grants and subscriptions, provision the stream or bucket, deploy and verify. Use when the user says subscribe to a NATS subject, consume NATS messages, event handler, message listener, NATS RPC, request/reply, service endpoint, durable consumer, at-least-once, JetStream, don't lose messages, replay a stream, batch worker, pull consumer, drain a queue, NATS KV, key/value store, config store, compare-and-swap, watch for changes, config reload, cache invalidation, fan out, broadcast, one-to-many, or anything else on NATS, JetStream, nats.io or Synadia. Pair with cosmonic-sandbox (the build and deploy loop) and cosmonic-nats-tuning (capacity, ack windows, payload sizing, the error catalogue).
license: Apache-2.0
compatibility: Requires Cosmonic Desktop with the `cosmonic` MCP server (wasmCloud v2.8+ host; the wasmcloud:nats plugin is always registered) and a NATS server the host can reach (default nats://127.0.0.1:4222, JetStream on for the JetStream/KV patterns). Rust builds target wasm32-wasip2 (the target Desktop provisions); Go builds need componentize-go v0.4.1 (see the cosmonic-go skill; the -airgap installer's Go module bundles it, otherwise install it yourself).
metadata:
  version: "1.0.1"
  author: Cosmonic
  upstream: "NATS-general references adapted from kaustavdm/nats-skill @ cbdda37 (MIT); templates and tuning from cosmonic-labs/nats-2.8-testing @ 5a437ac"
---

# Cosmonic NATS: `wasmcloud:nats` components

A component on Cosmonic Desktop does not open a NATS connection. It **imports**
`wasmcloud:nats@0.1.0` and **exports** a handler; the host owns the connection,
checks every subject against a deny-by-default grant, and pushes deliveries into
the component. This skill is the playbook for that shape. `references/` holds the
detail; `cosmonic-sandbox` owns the build → push → apply loop; `cosmonic-nats-tuning`
owns sizing and the error catalogue.

**Read in this order:** pick the pattern (§1) → know the surface (§2) → scaffold and
write the handler (§3) → declare the binding (§4) → run it (§5). Reach for
`cosmonic-nats-tuning` before touching capacity, ack, timeout, or `poolSize` settings.

## Hard rules

- **The guest never dials NATS.** No `nats.go`, no `async-nats`, no `servers:` in a
  manifest. Connection keys (`servers`, credentials, TLS) belong to the host — on
  Desktop that is Settings → Built-in plugins → NATS (`nats.yaml`). A manifest that
  carries one is refused at apply.
- **Grants are deny-by-default and separate.** `subject-allow` covers publish, request
  and core subscriptions; `stream-allow` covers JetStream reads; `bucket-allow` covers
  KV. Publishing to a subject does not grant reading the stream that captures it. Ship
  the minimum; widen only what the handler provably needs.
- **Streams, consumers and buckets are provisioned out-of-band** (`nats stream add`,
  `nats kv add`). The driver deliberately has no create/update/delete — a workload that
  cannot open a stream is missing provisioning or a grant, not code.
- **Async-only WASI p3.** Every function in the package is an `async func`. Rust needs
  wit-bindgen 0.60+ with `async-spawn`; Go needs componentize-go v0.4.1 (never standalone
  `wit-bindgen-go`, which silently emits zero functions). Always verify the export.
- **Prefer the JetStream consumer when unsure.** It paces delivery by acknowledgement,
  so a slow handler is throttled instead of overrun — the single biggest reliability
  difference in this interface.

## 1. Pick the pattern (seven templates, Rust and Go)

Three questions settle it: *does losing a message matter* (yes → JetStream)? *does the
caller wait for an answer* (yes → request/reply)? *is the state the point rather than
the message* (read/write → kv-store, react → kv-watcher)? `fan-out` is a composition
(one message becomes many), not a destination.

| Template id (`rust-nats-…` / `go-nats-…`) | Reach for it when the user says | Not this if |
|---|---|---|
| `core-subscriber` | "subscribe to a NATS subject", "consume NATS messages", "event handler", "message listener", "react to events", "NATS consumer" | losing a message matters — core NATS has no ack, no redelivery |
| `request-reply` | "NATS RPC", "request/reply", "service endpoint", "answer requests", "query service", "microservice on NATS" | the work outlasts the caller's timeout, or nobody awaits the reply |
| `jetstream-consumer` | "durable consumer", "at-least-once", "JetStream subscriber", "reliable delivery", "don't lose messages", "replay a stream" | you need exactly-once (be idempotent instead) or minimum latency |
| `jetstream-worker` | "batch worker", "pull consumer", "process a backlog", "drain a queue", "paced processing", "fetch messages" | the path is latency-critical; the fetch round-trip adds delay |
| `kv-store` | "NATS KV", "key/value store", "persist state", "config store", "compare-and-swap", "durable state" | you need queries, large blobs, or high write rates |
| `kv-watcher` | "watch for changes", "config reload", "cache invalidation", "react to state change", "change data capture" | you only need the value now (`get`), or you need queue semantics |
| `fan-out` | "fan out", "broadcast", "scatter", "one-to-many", "amplify", "notify many subscribers" | capacity and memory are unsized — resident memory is fan-out × payload |

Language: Rust unless the user asks for Go. A Go component is ~24× the size, ~13× the
per-replica memory, and **must never park on a Go runtime timer** (`time.Sleep`,
`time.After`, `context.WithTimeout` trap the instance). Per-pattern detail, the
shipped manifests and the local-NATS setup each needs: `references/patterns.md`.

## 2. Know the surface

Four interfaces, three handlers (`references/wasmcloud-nats.md` has every function
and error variant):

| Import | Gives the guest |
|---|---|
| `types` | `nats-message {subject, body, reply-to, headers}`, the `nats-error` variant (`denied`, `timeout`, `no-responders`, `max-payload-exceeded`, `revision-mismatch(current)`, `no-messages`, `limit-exceeded`, `already-settled`, `ack-owned-by-host`, …) |
| `core` | `publish(msg)`, `request(msg, timeout-ms) -> nats-message` (replies land on a per-workload inbox; `reply-to` on the request is ignored) |
| `jetstream` | `publish(msg) -> publish-ack`, `open-pull-consumer(stream, consumer) -> pull-consumer` (`fetch(batch, timeout-ms)`, `fetch-with-limits`, `info`), `get-by-sequence`, `scan`, `get-stream-info`, `list-stream-subjects`, `get-consumer-info`; `message-handle` (`message`, `sequence`, `delivery-count`, `ack`, `ack-sync`, `nak(delay)`, `in-progress`, `term`) |
| `kv` | `open(bucket) -> bucket` (`get`, `put`, `create`, `update(key, value, expected-revision)`, `delete`, `purge`, `keys(filter) -> key-page{keys, truncated}`, `history`, `status`) |

| Export | The host calls it for | Manifest key that drives it |
|---|---|---|
| `core-handler.handle-message(nats-message)` | each core subscription delivery, and each request (reply by publishing to `reply-to`) | `core-subscriptions: subj1,subj2` |
| `jetstream-handler.handle-message(message-handle)` | each JetStream push delivery | `jetstream-subscriptions: STREAM:filter[:policy[:queue]]` |
| `kv-handler.handle-event(bucket, entry)` | each put/delete/purge on a watched key | `kv-watches: bucket:filter` |

Settling a JetStream message is one-shot **on success**: an `ack`/`nak`/`term` the
server accepted retires the handle (`already-settled` on a repeat means the work was
done); a settle that failed on the wire leaves it usable, so retrying is correct. Under
`ack-mode: auto` the host acks on `Ok` and naks on `Err`/trap; an explicit settle
returns `ack-owned-by-host` (`in-progress` still works to extend ack-wait).

## 3. Scaffold and write the handler

Scaffold with `cosmonic_project_create(template="rust-nats-<pattern>", path=…,
name=…)` (or `cosmonic new rust-nats-<pattern>`); the id list is in
`cosmonic_template_list`. Every scaffold has the same shape:

```
.wash/config.yaml        build command + the wasmcloud:nats binding (workload.hostInterfaces)
deploy/workload.yaml     the same binding as a durable Workload manifest (image: built-in registry)
src/lib.rs | export_*/handler.go   START HERE — the handle_* function is the only thing to change
skills/<pattern>/SKILL.md          the pattern's own dial-in questions and pitfalls (read it first)
docs/tuning.md           the pattern's measured envelope; docs/nats-tuning.md the cross-pattern guide
scripts/e2e.sh           drives one message with the local `nats` CLI and asserts the effect
wit/world.wit + wit/deps/wasmcloud-nats-0.1.0/package.wit
```

Rust handler shape (wit-bindgen 0.60, `generate!({ path: "wit", world: "<pattern>", generate_all })`):

```rust
use exports::wasmcloud::nats::jetstream_handler::Guest as JetstreamHandler;
use wasmcloud::nats::jetstream::{self, MessageHandle};
use wasmcloud::nats::types::NatsMessage;

impl JetstreamHandler for Component {
    async fn handle_message(handle: MessageHandle) -> Result<(), String> {
        let msg = handle.message();               // borrow; sequence()/delivery_count() for dedup keys
        let out = NatsMessage { subject: "done.orders".into(), body: msg.body, reply_to: None, headers: None };
        jetstream::publish(out).await.map(|_| ()).map_err(|e| format!("publish failed: {e:?}"))
        // Ok(()) acks under ack-mode: auto; Err(..) naks → redelivery. Be idempotent.
    }
}
export!(Component);
```

Go handler shape (componentize-go bindings, module `wit_component`):

```go
func HandleMessage(handle *wasmcloud_nats_jetstream.MessageHandle) witTypes.Result[witTypes.Unit, string] {
    msg := handle.Message()
    out := wasmcloud_nats_types.NatsMessage{Subject: "done.orders", Body: msg.Body,
        ReplyTo: witTypes.None[string](), Headers: witTypes.None[[]wasmcloud_nats_types.HeaderEntry]()}
    if r := wasmcloud_nats_jetstream.Publish(out); r.IsErr() {
        return witTypes.Err[witTypes.Unit, string](fmt.Sprintf("publish failed: %s", ErrString(r.Err())))
    }
    return witTypes.Ok[witTypes.Unit, string](witTypes.Unit{})
}
```

Write handlers that are **reuse-safe**: with `poolSize` set, package-level state
survives across deliveries on a warm instance — treat it as a cache, never as
isolation and never as a place for per-message secrets. Keep them **panic-free**: a
trap poisons the instance and every call in flight on it.

## 4. Declare the binding

The scaffold's `.wash/config.yaml` and `deploy/workload.yaml` carry the same block;
Desktop lays the config one over the world-inferred interfaces for the dev workload
and the publish draft, so `cosmonic_dev_start` and `cosmonic_project_publish` work unchanged:

```yaml
hostInterfaces:
  - namespace: wasmcloud
    package: nats
    version: "0.1.0"
    interfaces: [types, jetstream, jetstream-handler]
    config:
      subject-allow: orders.>,done.>          # grants: deny-by-default ceilings
      stream-allow: ORDERS
      jetstream-subscriptions: ORDERS:orders.created:new:workers   # STREAM:filter:policy:queue
      ack-mode: auto                          # auto | manual
      # max-in-flight: "8"                    # bound on a small host; ≤ 1000 on Desktop
      # subscription-capacity: "1024"         # MESSAGES; size to the burst, not the rate
      # request-timeout-ms: "10000"           # publishes/acks at ≥1 MB
```

Rules the host enforces at apply: unknown keys fail (with the nearest known key named);
connection keys fail; under `workload_config: deny` a workload may only narrow a
declared grant. **Always give a JetStream push consumer the fourth field** (the queue
group) — it makes the consumer durable; an ephemeral one is server-deleted after 120 s
idle and delivery stops silently. The complete key set, by layer:
`references/wasmcloud-nats.md` §"Binding config".

## 5. Run it on Cosmonic Desktop

1. **A NATS server the host can reach.** Desktop dials `nats://127.0.0.1:4222` unless
   Settings → Built-in plugins → NATS says otherwise. Locally: `nats-server -js`, then
   provision what the manifest names (`nats stream add ORDERS --subjects 'orders.>'
   --defaults`, `nats kv add appkv`). Each scaffold's README lists its exact commands;
   `scripts/e2e.sh` asserts against a `RECEIPTS` stream on `done.>`.
2. **Build + run:** `cosmonic_dev_start` (the Builder's agent and `cosmonic dev`
   do the same). Tail `cosmonic_dev_logs`. A binding that cannot connect, lacks a
   grant, or names a missing stream **fails at start with the reason** — nothing
   retries silently; read the status and the events, then fix provisioning or the grant.
3. **Verify** with the scaffold's `scripts/e2e.sh`, or `nats pub` / `nats request` /
   `nats kv put` by hand and `cosmonic_logs_query` for the handler's side.
4. **Publish:** `cosmonic_project_publish` (confirm with the user) pushes to the built-in
   registry and returns a digest-pinned Workload that already carries the binding;
   `cosmonic_workload_apply` it. `deploy/workload.yaml` is the by-hand equivalent.

Verification that catches the two silent failures (a component that exports nothing;
a sync component):

```bash
wasm-tools component wit <out>.wasm | grep -q 'export wasmcloud:nats/<handler>@0.1.0'
wasm-tools print <out>.wasm | grep -qE 'async-lift|task-return'
```

## Build facts (what the templates use)

**Rust:** `wit-bindgen = { version = "0.60", features = ["async-spawn", "inter-task-wakeup"] }`,
`crate-type = ["cdylib"]`, `cargo build --release --target wasm32-wasip2` (the async
canonical ABI comes from wit-bindgen's generated code, not the target; the output IS a
component), `wkg.toml` overriding `wasmcloud:nats` to the vendored `wit/deps`.
`wasm32-wasip2` is the target Desktop's Preflight doctor provisions, so a clean machine
needs nothing extra; upstream measured the patterns on `wasm32-wasip1` builds — same
handler, same binding, and either target produces a component with the async exports.

**Go:** componentize-go **v0.4.1** — the version Cosmonic Desktop pins and its air-gap Go module
ships, with the async wit-bindgen-go compiled in. Install per the **cosmonic-go** skill (release
archive, never `go install` on Windows-on-ARM — use the `windows-amd64` build there;
cosmonic/desktop#433); it downloads a patched Go for async worlds, and the note
`does not support async operation; will use downloaded version` is expected. Build: `componentize-go -d ./wit -w <world> bindings --format && go mod tidy
&& componentize-go -d ./wit -w <world> build -o <name>.wasm` (the scaffold's
`.wash/config.yaml` does exactly this; `make build`/`make verify` too). Pass `-w`
explicitly: a dependency's `componentize-go.toml` can merge a world that mandates a
`wasi:http/incoming-handler` export. If you switch to the wasmCloud Go SDK
(`go.wasmcloud.dev/component` ≥ v0.1.3): `componentize-go build` only, never
`bindings` (it rewrites `go.mod`), and vendor the WASI deps. A Go component needs
~2.3 MiB of linear memory to instantiate — keep the host heap ≥ 4 MiB.

## When to open the other skill

`cosmonic-nats-tuning` — before changing `subscription-capacity`, `max-in-flight`,
`max-ack-pending`, `request-timeout-ms` or `poolSize`; for any payload ≥ 1 MB (size
the NATS server first); and for any of these symptoms: shedding / `backlog full` /
`host memory budget`, `DUPLICATES`, delivery that stops with no error, `nats: IO error`
storms, `kv put … didn't receive ack in time`, `already-settled`, `ack-owned-by-host`,
`limit-exceeded` on fetch, `async-lifted export failed to produce a result` (the Go
timer trap). It carries the measured numbers and the full error catalogue.

## References

- `references/patterns.md` — the seven patterns in depth: dial-in questions, shipped
  manifest config, local NATS setup, pitfalls, measured envelope per pattern.
- `references/wasmcloud-nats.md` — the driver: every WIT function and error variant,
  binding config keys by layer, grants, Desktop's `nats.yaml`, Rust/Go idioms.
- `references/nats-concepts.md` — NATS itself: subjects, core patterns, JetStream
  streams/consumers/retention, KV and object store semantics, connection, security
  overview, server config, subject mapping, monitoring, the `nats` CLI, common
  mistakes. (Adapted from kaustavdm/nats-skill; its client snippets are `nats.go`,
  which is the operator/tooling side here, never the guest.)
- `references/nats-jetstream.md`, `references/nats-security.md`,
  `references/nats-server.md` — the deep NATS references (stream/consumer config
  fields, NKey/JWT/TLS/auth callout, server config/clustering/leaf nodes/embedding).

Related skills: **cosmonic-sandbox** (the loop), **cosmonic-nats-tuning** (sizing).
