---
name: cosmonic-kafka
description: Build a Kafka-driven WebAssembly component for Cosmonic on the cosmonic:kafka@0.5.0 driver, where the host owns every client and the workload binding grants broker, credential, group and topics. Pick one of the four golden templates (handler consumer, the default; HTTP producer; pull service; transactional; Rust), write the code against the async WIT surface, declare the binding, create the topics, deploy and verify. Use when the user says produce to Kafka, publish to a topic, Kafka producer, webhook to Kafka, ingest gateway, consume Kafka, Kafka consumer, process each record, filter, validate, notify, sink, dead-letter, DLQ, consumer group, consume-transform-produce, enrich a topic, aggregate, batch, pipeline, seek, pause, rebalance, commit offsets, exactly-once, transactional, read-process-write, idempotent, payments, inventory, or anything else on Kafka, Redpanda, Confluent, MSK, Event Hubs, topics, partitions, brokers, librdkafka or rdkafka. Pair with cosmonic-sandbox (the build and deploy loop).
license: Apache-2.0
compatibility: Requires Cosmonic Desktop with the `cosmonic` MCP server to scaffold, build and publish (Rust with the wasm32-wasip2 target) and a host serving cosmonic:kafka@0.5.0 to run — Cosmonic Desktop compiles in Cosmonic Control's plugin-kafka (the default `kafka` feature), and Cosmonic Control 0.11+ on wasmCloud v2.9 serves the same. Needs a Kafka-protocol broker the host can reach, with the topics the manifest names already created.
metadata:
  version: "1.2.0"
  author: Cosmonic
  upstream: "templates from cosmonic-labs/awesome-cosmonic components/kafka @ f5326286 (the set Cosmonic Desktop vendors); binding rules from cosmonic/desktop daemon/docs/KAFKA.md and Control's plugin-kafka; the measured numbers from the cosmonic/kafka-demos campaign on the 0.3.0 driver, kept where the mechanism is unchanged"
---

# Cosmonic Kafka: `cosmonic:kafka` components

A component on Cosmonic does not link librdkafka. It **imports** `cosmonic:kafka@0.5.0`
and the host's Kafka plugin owns every client: the broker address, the credential, the
consumer group and the topic grant live in the workload's binding, and the component only
ever sees `producer` functions, a `Consumer` session, a `Transaction`, or the batches the
host hands its `handler`. This skill is the playbook for that shape. `references/` holds
the detail; `cosmonic-sandbox` owns the build → push → apply loop.

**Read in this order:** pick the pattern (§1) → know the surface (§2) → scaffold and
write the code (§3) → declare the binding (§4) → run it (§5) → the rules that keep it
running (§6).

## Hard rules

- **The guest never dials or configures Kafka.** No `rdkafka` crate, no `bootstrap.servers`
  in code, and no client config through the WIT at all: producer functions are called
  directly on the binding (there is no producer `open`, `close` or `flush`),
  `Consumer::open()` takes **no arguments**, `transaction::begin()` takes none. Broker,
  credential, group, offsets policy, limits and topics come only from the workload's
  `cosmonic:kafka` binding — a component cannot redirect itself or pick a group.
- **`topics` is the grant; `handler.topics` is the subscription.** The grant must contain
  every `handler.topics` subscription, the `dead-letter.topic`, and every topic a
  `producer`/`consumer`/`transaction` call names. Absent or empty grants **nothing**: a
  handler binding without `topics` is refused at bind. Grant exactly what the workload
  touches; never `*` wider than needed.
- **Handlers: `handler.group.id` and `dead-letter.topic` are both required.** The deploy
  fails without them; neither is derived (a derived group would collide across two
  installations of one manifest; without a DLQ a permanently failing record stalls its
  partition). A pull consumer's group is `consumer.group.id`; plain `group.id` is not a
  binding key. A `transaction` binding needs `transactional.id` and, for `send-offsets`,
  `transaction.group.id`.
- **Start with the handler — it can produce.** A handler that imports `producer` beside its
  `handler` export is the elastic consume-transform-produce shape; the host reuses one
  binding-scoped producer across instances. Reach for the pull service only when the code
  needs the consumer *session* (assign, pause, seek, rebalance events, pull pacing, its own
  commits), and for `transaction` only when outputs and offsets must commit atomically.
  Never open a pull consumer per invocation: it is a stateful group member (64 per component).
- **Async-only WASI p3.** Every function in the package is an `async func`. Rust needs
  wit-bindgen ≥ 0.58 with `async-spawn` + `inter-task-wakeup` on `wasm32-wasip2`. TinyGo
  cannot bind it; there is no Go set (componentize-go is the candidate, not shipped).
- **Malformed input is `permanent`, never a panic.** A trap is retried like a transient
  error; five consecutive traps on one record dead-letter it, and until then that
  partition makes no progress.

## 1. Pick the pattern (four templates, Rust)

Two questions settle it: *does the work start from an HTTP request or from a record*
(request → `http-producer`)? For record-driven work: *does the code need to own the
consumer session or a transaction* (no → `handler-consumer`, the default — including when
it produces; session control → `pull-service`; atomic outputs + offsets → `transactional`).

| Template id (`rust-kafka-…`) | Shape | Guarantee | Reach for it when the user says | Not this if |
|---|---|---|---|---|
| `handler-consumer` (**start here**) | host pushes a partition-ordered batch of records to `handle`; add `producer` to publish | at-least-once; DLQ on `permanent` | "consume Kafka", "process each record", "event handler", "filter", "validate", "notify", "sink", "consume → transform → produce", "enrich a topic", "serverless", "scale to zero" | you need assign/pause/seek/rebalance events or your own commit policy (→ pull service), or outputs and offsets must commit in one transaction (→ transactional) |
| `http-producer` | HTTP request → `send` / `send-batch` on the binding's producer | broker ack per record (`acks` configurable) | "webhook to Kafka", "API that writes to a topic", "ingest gateway", "UI events onto a topic", "produce from a request" | the work is driven by Kafka records rather than HTTP |
| `pull-service` | long-running `wasi:cli/run` service owns one consumer session + uses the host's producer, batches, commits explicitly | at-least-once; your own DLQ routing | "seek", "pause/resume", "assign partitions", "react to rebalances", "commit offsets myself", "custom pull pacing", "long-lived session" | ordinary elastic processing (the handler is less code, scales without touching group membership, and can produce), or you need exactly-once |
| `transactional` | pull service + a second, named `transaction` binding: outputs and input offsets commit atomically | exactly-once (read-process-write) | "exactly-once", "transactional", "payments", "inventory", "ledger", "dedup-sensitive", "read_committed" | throughput matters more than duplicates (a txn round trip per batch), or side effects leave Kafka — a DB write is not covered; then at-least-once + idempotent writes is the honest design |

Per-pattern detail, the shipped manifests and the measured numbers: `references/patterns.md`.

## 2. Know the surface

Five interfaces, one export (`references/cosmonic-kafka.md` has every function and the
`error` record):

| Import | Gives the guest |
|---|---|
| `types` | `consumed-record {topic, partition, offset, key?, value?, headers, timestamp?, timestamp-type, leader-epoch?}`, `produce-record {partition?, key?, value?, headers, timestamp?}`, `produce-ack {partition, offset, timestamp?}`, `partition-ref`, `partition-offset`, `partition-result`, `watermarks`, `error {code, message, fatal, retriable, txn-requires-abort}` (`code` is every librdkafka `rd_kafka_resp_err_t`, plus `unknown-error-code(s32)`). Bound whenever any other interface is; needs no `interfaces:` entry |
| `producer` | **free functions on the binding's host-owned client** — `send(topic, record) -> produce-ack`, `send-batch(topic, records) -> list<result<ack, error>>` (per-record outcomes), `send-stream(topic, stream) -> future<result>`, `partition-count`, `watermark-offsets`. No `open`, no `flush`: each call reports its own delivery outcome |
| `consumer` | `consumer.open()` (no arguments — the binding fixes everything), `subscribe(topics)`, `records() -> (stream<consumed-record>, future<result>)` (callable once), `commit(offsets)` (`[]` = the stored positions), `committed`, `position`, `seek(partitions, position)`, `pause`/`resume`, `assign`/`incremental-assign`, `rebalances() -> stream<rebalance-event>`, `watermark-offsets`, `offsets-for-times`, `close` |
| `transaction` | its own binding, never shared with `producer`: `begin() -> transaction` (fences the previous holder of the binding's `transactional.id`; one active transaction per binding), then on the resource `send`, `send-batch`, `send-stream`, `send-offsets(offsets)` (the group is the binding's `transaction.group.id`), `commit`, `abort`; plus `partition-count`, `watermark-offsets` |

| Export | The host calls it for | Manifest keys that drive it |
|---|---|---|
| `handler.handle(list<consumed-record>) -> result<option<offset>, handler-error>` | each batch from ONE partition, in offset order — up to `handler.batch.size` (default 100, max 10000) and ~1 MiB | `handler.topics`, `handler.group.id`, `dead-letter.topic`, `topics` (must cover both), `handler.batch.size`, `auto.offset.reset` |

Handler return semantics (at-least-once): `Ok(None)` = the whole batch handled, the host
stores past its last record; `Ok(Some(offset))` = handled through that record (its OWN
offset, not the next), the host stores past it and redelivers the rest; `Err(transient)` =
nothing handled, rewind and redeliver (backing off 100 ms → 30 s) indefinitely;
`Err(permanent)` = a single-record batch goes to `dead-letter.topic` and the partition
advances — on a longer batch the host redelivers the FIRST record alone so the verdict
lands on one record. **Prefer `Ok(Some(..))` over an error once any record succeeded**: it
keeps the work already done.

## 3. Scaffold and write the code

Scaffold with `cosmonic_project_create(template="rust-kafka-<pattern>", path=…, name=…)`
(or `cosmonic new rust-kafka-<pattern>`); the ids are in `cosmonic_template_list`. Every
scaffold has the same shape:

```
.wash/config.yaml        build command + the cosmonic:kafka binding (workload.hostInterfaces)
                         + workload.environment.config (the service patterns' topics/batch size)
workload.yaml            the same binding as a Cosmonic Workload (image: built-in registry)
deploy/workload-deployment.yaml   the Kubernetes WorkloadDeployment (Control; prebuilt image)
src/lib.rs               START HERE — the header comment states the pattern's semantics
wit/world.wit + wit/deps/cosmonic-kafka-0.5.0/package.wit   vendored; no registry fetch to build
wkg.lock + wkg-registries.toml   to re-fetch the WIT (`WKG_CONFIG_FILE=./wkg-registries.toml wash wit fetch`)
```

Handler shape (wit-bindgen 0.58, `generate!({ world: "kafka-handler-consumer", generate_all })`;
the world name stays the pattern's — only the crate is renamed):

```rust
use bindings::cosmonic::kafka::types::ConsumedRecord;
use bindings::exports::cosmonic::kafka::handler::{Guest as Handler, HandlerError};

impl Handler for Component {
    async fn handle(records: Vec<ConsumedRecord>) -> Result<Option<i64>, HandlerError> {
        let mut handled: Option<i64> = None;
        for rec in &records {
            let Some(value) = rec.value.as_deref() else { handled = Some(rec.offset); continue }; // tombstone
            match process(value) {                                   // your work; never panic
                Ok(()) => handled = Some(rec.offset),
                Err(Bad) => return handled.map_or(Err(HandlerError::Permanent(Some("bad input".into()))), |o| Ok(Some(o))),
                Err(Retry) => return handled.map_or(Err(HandlerError::Transient(None)), |o| Ok(Some(o))),
            }
        }
        Ok(None)
    }
}
```

To publish from a handler, add `import cosmonic:kafka/producer@0.5.0;` to `wit/world.wit`,
`producer` to the binding's `interfaces`, the output topic to `topics`, and call
`producer::send(topic, record).await` — the host's client, no open.

Service shape (`wasi:cli/run`; one consumer session, the binding's producer, explicit commit):

```rust
use bindings::cosmonic::kafka::{consumer::Consumer, producer};

let consumer = Consumer::open().await?;                 // group, offsets policy: from the binding
consumer.subscribe(vec![in_topic]).await?;
let (mut records, terminal) = consumer.records().await?; // keep `terminal` alive for the loop
let mut pending = Vec::with_capacity(batch);
while let Some(rec) = records.next().await {
    match transform(&rec) { Ok(out) => pending.push(out), Err(why) => { producer::send(dlq.clone(), dead(&rec, why)).await?; } }
    if pending.len() >= batch {
        let outcomes = producer::send_batch(out_topic.clone(), std::mem::take(&mut pending)).await?;
        if outcomes.iter().all(|o| o.is_ok()) { consumer.commit(Vec::new()).await?; } // stored positions, after the acks
    }
}
```

Exactly-once wraps each batch through the **named** `transaction` binding
(`import transaction: cosmonic:kafka/transaction@0.5.0;` in the world):
`transaction::begin()` → `txn.send_batch(out_topic, outputs)` →
`txn.send_offsets(one_past_last_per_partition)` (carry `leader_epoch` through) →
`txn.commit()`; on any failure `txn.abort()` and the uncommitted input offsets redeliver the
batch. Check `error.txn_requires_abort` / `error.fatal` (fatal ⇒ the host retires the native
client; a fresh `begin` re-fences). Env vars (`IN_TOPIC`, `OUT_TOPIC`, `DLQ_TOPIC`,
`BATCH_SIZE`) come from `localResources.environment.config` — read them with
`std::env::var`, not a generated `wasi:cli/environment` binding, so the world adds no
second `wasi:cli` version.

Write handlers that are **reuse-safe** (with `poolSize` set, package-level state survives
across batches on a warm instance — a cache, never isolation) and **panic-free**.

## 4. Declare the binding

The scaffold's `.wash/config.yaml` and `workload.yaml` carry the same block; Desktop lays
the config over the world-inferred interfaces for the dev workload and the publish draft:

```yaml
hostInterfaces:
  - namespace: cosmonic
    package: kafka
    version: "0.5.0"
    interfaces: [handler]                 # what the world imports/exports: handler | producer | consumer | transaction (types is implied)
    config:
      bootstrap.servers: 127.0.0.1:9092   # on Desktop: overrides any kafka.yaml default; omit to inherit one
      broker.address.family: v4           # keeps librdkafka off ::1 when the broker advertises localhost
      handler.topics: demo.events         # the subscription (handlers)
      topics: demo.events,demo.events.dlq # the GRANT — must cover the subscription, the DLQ and every produce target
      handler.group.id: demo-events-handler   # required, stable across redeploys
      dead-letter.topic: demo.events.dlq      # required
      auto.offset.reset: earliest
      # handler.batch.size: "100"         # 1..10000, ≤ ~1 MiB per call
    # secretFrom: [kafka-credentials]     # Desktop: a ref registered as SASL_PASSWORD arrives as sasl.password; Control: a Secret with sasl.username / sasl.password / ssl.ca.pem …
```

A pull service declares `interfaces: [consumer, producer]` with `consumer.group.id`,
`auto.offset.reset`, `enable.auto.commit: "false"` and a grant covering input, output and
DLQ. Exactly-once is **two entries** on one workload — the unnamed `[consumer]` one, plus a
named one the world imports as `transaction:`:

```yaml
  - name: transaction
    namespace: cosmonic
    package: kafka
    version: "0.5.0"
    interfaces: [transaction]             # never beside `producer` on the same binding
    config:
      bootstrap.servers: 127.0.0.1:9092
      topics: demo.events,demo.enriched   # the output topic AND every input topic whose offsets it enlists
      transactional.id: demo-pipeline-txn-1   # one STABLE id per live instance — replicas: 1
      transaction.group.id: demo-txn-g1       # = the consumer binding's consumer.group.id
```

Rules the host enforces: **the binding is the whole client config** — a guest passes
nothing, so nothing can be overridden. Every binding that grants `producer`, `consumer` or
`transaction` needs a broker: on Desktop from this entry or from the host's
`<state_dir>/kafka.yaml` default (a base the entry overrides key by key; `config` for the
unnamed binding, `bindings.<name>` for a label; `secret_from` names refs the host resolves at
boot), and a binding with neither is refused. The keys that would hand the *host process* a
capability are refused to a workload on every host — anything loading native code
(`plugin.library.paths`, `ssl.engine.location`, `ssl.providers`), reading a host file by
path (`ssl.*.location`, `https.ca.location`, `sasl.kerberos.keytab`, the
`sasl.oauthbearer.assertion.*.file` keys), running a command or reaching a host-chosen URL
(`sasl.kerberos.kinit.cmd`, `sasl.oauthbearer.token.endpoint.url`), or weakening/flooding
the host (`enable.ssl.certificate.verification`, `ssl.endpoint.identification.algorithm`,
`debug`, `statistics.interval.ms`). Credentials are yours: pass TLS material inline as
`ssl.ca.pem` and friends through `config`/`secretFrom`, never as file paths. librdkafka
alias pairs (`metadata.broker.list` = `bootstrap.servers`, …) are folded to one spelling,
so an alias cannot dodge a rule. Where an enterprise machine layer sets an egress
allow-list, every broker must be on it. One workload attaches to several Kafkas with one
entry per `(implements <name>)` label. Egress stays `allowedHosts: []` — the plugin dials
the broker, the component does not.

## 5. Run it

1. **A broker the host can reach, with the topics the manifest names** — the
   plugin does not create topics. Any Kafka-protocol server on `127.0.0.1:9092` locally
   (`docker run -d -p 127.0.0.1:9092:9092 apache/kafka:3.9.0`, `redpanda start --mode
   dev-container`); then `kafka-topics --bootstrap-server 127.0.0.1:9092 --create --topic
   demo.events` (each scaffold's README lists its exact set; the handler also needs the DLQ,
   the service patterns the output and DLQ topics).
2. **Build + run:** `cosmonic_dev_start` (the Builder's agent and `cosmonic dev` do the same);
   tail `cosmonic_dev_logs`. A bad binding is refused with a sentence that names the key,
   never a value: Desktop's apply guard rejects a host-only key or an off-allow-list broker
   (400; `cosmonic_workload_validate` says the same), and the plugin's bind checks — no
   broker anywhere, a grant that misses the subscription or DLQ, a missing group id — fail
   the start **permanently** (Failed, no retry; fix and re-apply). Read the sentence before
   touching the code. A start that says `requires cosmonic:kafka which this host does not
   provide` means a daemon built without the `kafka` feature; do not retry in a loop.
3. **Verify** end to end, never from the host log alone: produce one probe record
   (`kafka-console-producer` / `rpk topic produce`), then read the output or DLQ topic and
   the consumer group's **lag** (`rpk group describe <group>`, `kafka-consumer-groups
   --describe`). Lag == 0 is "caught up"; offsets that stopped moving is not evidence of
   anything.
4. **Publish:** `cosmonic_project_publish` (confirm with the user) pushes to the built-in
   registry and returns a digest-pinned Workload that carries the binding;
   `cosmonic_workload_apply` it. `workload.yaml` is the by-hand equivalent.

Verification that catches the silent build failures (a component that exports nothing;
a sync component; a wrong interface list):

```bash
wasm-tools component wit <out>.wasm | grep -E 'cosmonic:kafka/(handler|producer|consumer|transaction)@0.5.0'
wasm-tools print <out>.wasm | grep -qE 'async-lift|task-return'
# then make each binding's `interfaces:` list match those lines (a named import → a `name:` entry)
```

## 6. Rules that keep it running

- **Scaling** = partitions × replicas, for handlers and pull services alike; a partition
  is only ever assigned to one group member. Per replica a handler's useful concurrency is
  `min(assigned partitions, 64, poolSize × maxConcurrency)`; growing the pool adds no group
  member and moves no partition (the templates ship `poolSize: 32`, `maxConcurrency: 1` —
  raise `maxConcurrency` only for code that is safe under overlapping calls on one
  instance). Add `replicas` for availability or more group members.
- **A slow handler wants `max.poll.interval.ms`, not a smaller pool.** The per-call
  deadline derives from it (the interval less a minute — ten minutes by default) and
  covers the whole batch; the host serves the group's poll timer independently.
- **Rebalancing is `cooperative-sticky`** by default; every member of a group must agree,
  so never mix strategies across deployments sharing a `handler.group.id`.
- **Throughput vs latency in a service** is `BATCH_SIZE` (default 1, capped at 100, no
  batch timer — a partial batch waits for the next record or the end of the stream). Per-record
  send+await is a broker round trip each (~126 records/s measured); batches of 100 move
  thousands per second. Raise it only for a steady stream.
- **Never drain an unbounded `records()` stream inside one request-scoped call** — it
  stalls silently past ~10–12k records. Chunk, or move it into a service. `records()` is
  callable once per consumer, and reading it is what keeps the group heartbeat alive.
- **Read the `error` record, not just `code`:** `retriable` says retry; `fatal` on the
  producer means the host retires the native client (a consumer must be closed and
  reopened); `txn-requires-abort` means abort the transaction and reprocess.
  `not-leader-for-partition` is "refresh metadata and retry", not "give up".
- **After a broker restart that changed its identity** expect minutes of silent
  no-delivery, then automatic recovery (10 s backoff loop) — alert on consumer lag, send a
  probe record before trusting the pipeline. The symptom catalogue, log line by log line:
  `references/troubleshooting.md`.

## References

- `references/patterns.md` — the four patterns in depth: the shipped manifest of each, the
  env contract of the services, the operational limits, and the pitfalls per pattern.
- `references/cosmonic-kafka.md` — the driver: every WIT function, the `error` record and
  how to read it, the config keys by owner (driver / workload / host-refused / bounded), the
  built-in limits, Desktop's `kafka.yaml`, and Rust idioms for streams and responses.
- `references/troubleshooting.md` — symptom → cause → fix, including the failures with no
  log line at all.

Related skills: **cosmonic-sandbox** (the loop), **cosmonic-nats** (the same host-owned
shape on NATS — pick NATS when the user wants subjects, JetStream or KV rather than topics).
