# The `wasmcloud:nats@0.1.0` driver

What a component gets when it imports `wasmcloud:nats`, what the host enforces, and
where every knob lives. The authoritative text is the package's WIT
(`wit/deps/wasmcloud-nats-0.1.0/package.wit` in every scaffold); this is the working
summary. Cosmonic Desktop serves the package through wash-runtime's `wasmcloud-nats`
plugin (wasmCloud v2.8+), the same driver Cosmonic Control and `wash dev` ship. It is
registered on every host, always on, and inert until a workload imports the package.

## The model

- **The host owns the connection.** It opens one connection per workload binding, from
  the daemon process, to whatever `servers` the binding resolves to. On Desktop that is
  the address in Settings → Built-in plugins → NATS (`nats.yaml`, default
  `nats://127.0.0.1:4222`), never something a manifest names.
- **Grants are deny-by-default and separate.** Every subject, stream and bucket is
  checked against the workload's grant before anything reaches the server; a name
  outside it returns `denied` and never leaves the host. `subject-allow` covers
  publish/request/core subscriptions, `stream-allow` covers JetStream stream reads,
  `bucket-allow` covers KV. Stored messages are checked against `subject-allow` too —
  `stream-allow` on its own reaches nothing readable.
- **Reserved space** is denied regardless of grant: `$JS.`, `$SYS.`, `$KV.`, `$OBJ.`,
  the plugin's own delivery plane, and the lattice's control subjects
  (`runtime.host.*`, `runtime.operator.*`).
- **Lifecycle is out-of-band.** No stream/consumer/bucket create, update or delete —
  provision with the `nats` CLI or deployment tooling. Introspection is served
  (`get-stream-info`, `get-consumer-info`, `list-stream-subjects`, `status`).
- **Everything is `async func`**, so a component may keep many publishes, requests and
  fetches in flight and a handler may await imports while the host keeps delivering.

## Interfaces

### `types`

```wit
record nats-message { subject: string, body: list<u8>, reply-to: option<string>, headers: option<list<header-entry>> }
record header-entry { name: string, value: string }   // printable ASCII names without ':'; no CR/LF in values
```

`nats-error` variants and what to do with them:

| Variant | Meaning | Do |
|---|---|---|
| `connection(string)` | transport error on a live connection | retry with backoff; check the server |
| `disconnected` | no live connection bound to this workload | the host is reconnecting; retry later |
| `timeout(string)` | request timed out | raise the caller's timeout (5–10 s at ≥1 MB) or check the responder |
| `no-responders` | nobody subscribed to the request subject | not a timeout — retrying now fails the same way |
| `denied(denial)` | outside the grant or reserved (`reason`: `reserved` / `not-granted` / `wildcard-not-allowed`; `target`: `subject` / `stream` / `bucket` / `message(seq)`) | widen the grant in the manifest (an operator matter under `deny`), never work around it |
| `max-payload-exceeded(u64)` | payload (headers included) over the server's `max_payload`; value is the limit | chunk, or raise `max_payload` on the server |
| `invalid-header(string)` | header not representable on the wire | fix the name/value |
| `jetstream(string)` | JetStream error detail | read it; usually provisioning or limits |
| `not-found(string)` | stream/bucket/consumer doesn't exist | provision it (`nats stream add`, `nats kv add`, `nats consumer add`) |
| `unsupported-by-server(string)` | server too old; names the minimum | upgrade the server (`list-stream-subjects` needs 2.7.2+) |
| `key-not-found` | absent, deleted or purged key | typed status, not a failure |
| `revision-mismatch(u64)` | CAS conflict; carries the *current* revision | retry with the carried revision, no re-read |
| `no-messages` | pull fetch returned empty within the timeout | the consumer had nothing; loop or stop |
| `limit-exceeded(string)` | fetch refused before delivery: over the consumer's provisioned limits (size against `info`) OR earlier fetched handles hold the binding's memory budget (drop them) | retrying unchanged fails the same way |
| `already-settled` | an ack/nak/term the server accepted already retired the handle | treat as success — the work was done |
| `ack-owned-by-host` | binding runs `ack-mode: auto`; the host settles | drop the explicit settle, or switch to `manual`; `in-progress` works either way |
| `unexpected(string)` | anything else (e.g. `fetch` with `batch: 0`) | read the string |

### `core`

- `publish(msg) -> result<_, nats-error>` — fire-and-forget; resolves when written to
  the connection, not when a subscriber saw it.
- `request(msg, timeout-ms) -> result<nats-message, nats-error>` — one reply. Replies
  arrive on a **per-workload inbox**; `msg.reply-to` is ignored. Many requests may be
  outstanding at once.

### `jetstream`

- `publish(msg) -> publish-ack {stream-name, sequence, duplicate}` — idempotent via a
  `Nats-Msg-Id` header within the stream's duplicate window; `reply-to` ignored.
- `open-pull-consumer(stream, consumer) -> pull-consumer` — the consumer must exist.
  - `fetch(batch, timeout-ms) -> fetched-batch {messages, stop}` (`batch ≥ 1`;
    `no-messages` if the timeout elapses empty).
  - `fetch-with-limits(batch, max-bytes, timeout-ms)` — byte-bounded; a batch cut short
    comes back with `stop: byte-limit`, not as a drained consumer.
  - `info() -> consumer-info` — `max-request-batch`, `max-request-max-bytes`,
    `max-waiting`, `max-ack-pending`, `max-deliver`, `ack-wait-ms`, `num-ack-pending`,
    `num-pending`, `num-redelivered`, filters.
- `message-handle` (from push delivery or a fetch): `message()`, `sequence()`,
  `delivery-count()` (1 on first delivery), `ack()` (fire-and-forget), `ack-sync()`
  (waits for server confirmation — the only way to know a delivery will not repeat),
  `nak(delay-ms?)`, `in-progress()` (extend ack-wait; repeatable), `term()` (never
  redeliver). Settling is one-shot **on success** only.
- `get-by-sequence(stream, seq) -> stored-message`, `scan(stream, start-seq, max-count)`
  (stateless replay; messages outside the subject grant are skipped and do not count),
  `get-stream-info(stream)`, `list-stream-subjects(stream, filter)` (≤ 1000; `denied`
  rather than an empty list when nothing matching is granted), `get-consumer-info`.

### `kv`

- `open(bucket) -> bucket` (`not-found` / `denied`).
- `bucket`: `get(key) -> entry {key, value, revision, created-at-unix-nanos, operation}`,
  `put(key, value) -> revision`, `create(key, value)` (only if absent),
  `update(key, value, expected-revision)` (CAS), `delete(key)` (tombstone, keeps
  history), `purge(key)` (removes history), `keys(filter) -> key-page {keys, truncated}`
  (≤ 1000 per page; `>` = all), `history(key) -> list<entry>`, `status() ->
  bucket-status {bucket, values, history, ttl-seconds, bytes}`.

### Handlers (exports)

| Interface | Function | Driven by | Settle |
|---|---|---|---|
| `core-handler` | `handle-message(nats-message) -> result<_, string>` | `core-subscriptions`; also request/reply (reply by publishing to `reply-to`) | none (core has no ack) |
| `jetstream-handler` | `handle-message(message-handle) -> result<_, string>` | `jetstream-subscriptions` | `ack-mode: auto` → host acks on ok, naks on error/trap; `manual` → the handle is the only path |
| `kv-handler` | `handle-event(bucket: string, entry) -> result<_, string>` | `kv-watches` | none |

Returning `Err` from a JetStream handler under `auto` naks (redelivery); returning
without settling under `manual` is not an error but stalls the consumer for the full
ack-wait.

## Binding config

The keys, grouped by who sets them. Config keys are validated against a closed schema —
an unknown key **fails deployment** with the nearest known key named, rather than
being ignored.

| Layer | Keys | Who sets them on Desktop |
|---|---|---|
| **Connection / identity** | `servers`, `name`, `jetstream-domain`, `inbox-prefix`, `creds` (`creds-file`), `jwt`, `nkey-seed` (`nkey`), `username` (`user`), `password`, `token`, `tls-ca`, `tls-cert`, `tls-key`, `tls-first` | **the host only** — `nats.yaml` (Settings → Built-in plugins → NATS). A manifest carrying one is refused at apply, again on the resolved config at every start (so `configFrom`/`secretFrom` cannot route around it), and on the Control-pushed path. |
| **Grants** (ceilings) | `subject-allow`, `stream-allow`, `bucket-allow` (comma-separated; NATS wildcards `*` and `>` allowed in grants and subscriptions, never in a publish/request subject) | the workload under Desktop's default `workload_config: allow`; under `deny` the host declares the ceiling and a workload may only narrow it |
| **Behaviour** | `core-subscriptions` (comma-separated subjects), `jetstream-subscriptions` (`STREAM:filter[:policy[:queue]]`, comma-separated; policy `new` \| `all` \| `last` \| `last-per-subject`), `kv-watches` (`bucket:filter`), `component`, `ack-mode` (`auto` \| `manual`), `max-in-flight`, `max-ack-pending`, `max-deliver`, `subscription-capacity` (messages), `subscription-capacity-bytes` (default 32 MiB), `request-timeout-ms` | the workload |

Values are strings (`"8"`, not `8`). Behaviour defaults and the derivations
(`max_ack_pending = min(max-in-flight × 2, subscription-capacity, capacity_bytes ÷
per_message_bytes)`, floored at 16; host-wide backlog ceiling = guest memory ÷ 4) are
in the `cosmonic-nats-tuning` skill.

A workload manifest (and the identical `workload.hostInterfaces` block in
`.wash/config.yaml`, which Desktop lays over the world-inferred interfaces for the dev
workload and the publish draft):

```yaml
spec:
  hostInterfaces:
    - namespace: wasmcloud
      package: nats
      version: "0.1.0"
      interfaces: [types, jetstream, jetstream-handler]
      # name: orders                       # ← (implements orders): a named binding from nats.yaml
      config:
        subject-allow: load.>,done.js-sink.>
        stream-allow: LOAD
        jetstream-subscriptions: LOAD:load.push.>:new:workers
        ack-mode: auto
  components:
    - name: orders-consumer
      image: oci.localhost:8200/apps/orders-consumer:0.1.0
      poolSize: 1          # instance reuse: every delivery path; unset/0 = fresh instance per delivery
      maxInvocations: 0
      localResources:
        allowedHosts: []   # NATS traffic is the host binding, not HTTP egress
```

A binding that cannot connect, lacks a grant, or names a missing stream/bucket **fails
at start** with the reason in its status and the event stream; nothing retries silently.

## Desktop's side: `nats.yaml`

`GET`/`PUT /v1/nats`, Settings → Built-in plugins → NATS. Baked into the host at boot
(changes apply on the next daemon restart):

```yaml
servers: nats://127.0.0.1:4222   # where a binding that names no servers dials
workload_config: allow           # allow (default, dev host) | warn | deny (Control's posture)
# config: {}                     # base layer: grants under deny, host-wide creds/tls paths
# secret_from: []                # secret refs (token/password/nkey-seed/jwt) merged at boot
# bindings: {}                   # named bindings: (implements <name>) in a manifest
# host_owned_keys: []            # extra keys claimed for the host under deny
```

`COSMONIC_NATS_URL` overrides `servers` for one boot. Credential *values* are refused
as literal `config` keys — secrets are references (Settings → Secrets), never values.
A workload that needs a different cluster or different credentials gets a **named
binding** the operator declares, imported with `name: <binding>` on the hostInterface.
`POST /v1/nats/test` dials a declaration from the daemon and reports what answered.

Memory: the plugin is handed the engine's guest-memory budget and bounds what every
core subscription on the host may hold between them; past the ceiling deliveries shed
with `reason="host memory budget"`. **Keep `max-in-flight` ≤ 1000 on Desktop** — the
engine's pooling allocator admits 1000 concurrent core instances and deliveries above
it fail (`maximum concurrent limit of 1000 for core instances reached`) rather than
queue; size `subscription-capacity[-bytes]` instead, which queues.

## Guest idioms

Rust (wit-bindgen 0.60 `generate!` with `generate_all`; the scaffold's `world` name is
the pattern):

```rust
use wasmcloud::nats::{core, jetstream, kv};
use wasmcloud::nats::types::NatsMessage;

// request/reply: answer on the caller's inbox
async fn reply(msg: NatsMessage) -> Result<(), String> {
    let Some(reply_to) = msg.reply_to else { return Ok(()) };     // plain publish: nothing to answer
    core::publish(NatsMessage { subject: reply_to, body: b"ok".to_vec(), reply_to: None, headers: None })
        .await.map_err(|e| format!("reply failed: {e:?}"))
}

// KV compare-and-swap with the carried revision
async fn bump(bucket: &kv::Bucket, key: &str) -> Result<u64, String> {
    let cur = bucket.get(key.to_string()).await.map_err(|e| format!("{e:?}"))?;
    bucket.update(key.to_string(), next(&cur.value), cur.revision).await.map_err(|e| format!("{e:?}"))
}

// pull worker: bounded batches, settle, drop handles
let puller = jetstream::open_pull_consumer("LOAD".into(), "workers".into()).await.map_err(|e| format!("{e:?}"))?;
let batch = puller.fetch(4, 5_000).await.map_err(|e| format!("{e:?}"))?;   // 4 at 1–2 MB, 1–5 at 5 MB
for h in &batch.messages { h.ack().await.map_err(|e| format!("{e:?}"))?; }
```

Go (componentize-go generated bindings; `witTypes` is `go.bytecodealliance.org/pkg/wit/types`):

```go
r := wasmcloud_nats_core.Publish(wasmcloud_nats_types.NatsMessage{
    Subject: replyTo, Body: []byte("ok"),
    ReplyTo: witTypes.None[string](), Headers: witTypes.None[[]wasmcloud_nats_types.HeaderEntry](),
})
if r.IsErr() { return witTypes.Err[witTypes.Unit, string](ErrString(r.Err())) }
```

Never `time.Sleep` / `time.After` / `context.WithTimeout` inside a Go handler: the
instance traps with `async-lifted export failed to produce a result`. Await
`wasi:clocks/monotonic-clock@0.3.0` (the wasmCloud Go SDK's `sleep` package) instead.

## Provisioning cheat-sheet (`nats` CLI)

```bash
nats-server -js -m 8222                                       # JetStream + monitoring
nats stream add LOAD --subjects 'load.>' --defaults           # a stream the manifest names
nats stream edit LOAD --max-msg-size=8388608 --force          # ≥1 MB payloads: ack window derives from it
nats consumer add LOAD workers --pull --deliver all --ack explicit --filter 'load.pull.>'   # for open-pull-consumer
nats kv add appkv                                             # a bucket the manifest names
nats stream add RECEIPTS --subjects 'done.>' --defaults       # what the scaffolds' e2e asserts on
nats pub load.push.msg 'hello' ; nats request svc.echo 'hi' --timeout 5s ; nats kv put appkv k v
nats consumer ls LOAD ; nats stream info LOAD                 # orphaned ephemerals, sequences
```
