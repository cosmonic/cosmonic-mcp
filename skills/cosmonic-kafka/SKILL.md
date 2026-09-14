---
name: cosmonic-kafka
description: Build a Kafka-driven WebAssembly component for Cosmonic on the cosmonic:kafka@0.3.0 driver. Pick one of the four golden templates (HTTP producer, handler consumer, pull service, transactional pipeline; Rust), write the handler or service loop against the async WIT surface, declare the broker, topic grant and handler keys in the workload binding, create the topics, deploy and verify. Use when the user says produce to Kafka, publish to a topic, Kafka producer, webhook to Kafka, ingest gateway, consume Kafka, Kafka consumer, process each record, filter, validate, notify, sink, dead-letter, DLQ, consumer group, consume-transform-produce, enrich a topic, aggregate, batch, commit offsets, pipeline, exactly-once, transactional, read-process-write, idempotent, payments, inventory, or anything else on Kafka, Redpanda, Confluent, MSK, Event Hubs, topics, partitions, brokers, librdkafka or rdkafka. Pair with cosmonic-sandbox (the build and deploy loop).
license: Apache-2.0
compatibility: Requires Cosmonic Desktop with the `cosmonic` MCP server to scaffold, build and publish (Rust with the wasm32-wasip2 target) and a host serving cosmonic:kafka@0.3.0 to run — Cosmonic Control on wasmCloud v2.8+ today, Cosmonic Desktop once cosmonic/desktop#389 lands (until then a start fails cleanly with "requires cosmonic:kafka which this host does not provide"). Needs a Kafka-protocol broker the host can reach.
metadata:
  version: "1.1.0"
  author: Cosmonic
  upstream: "templates and every measured number from cosmonic-labs/awesome-cosmonic components/kafka @ 77d3153; troubleshooting from the cosmonic/kafka-demos campaign"
---

# Cosmonic Kafka: `cosmonic:kafka` components

A component on Cosmonic does not link librdkafka. It **imports** `cosmonic:kafka@0.3.0`
and the host's Kafka plugin owns every client: the broker address, the credential and the
topic grant live in the workload manifest, the component only ever sees `Producer`,
`Consumer` and the records the host hands it. This skill is the playbook for that shape.
`references/` holds the detail; `cosmonic-sandbox` owns the build → push → apply loop.

**Read in this order:** pick the pattern (§1) → know the surface (§2) → scaffold and
write the code (§3) → declare the binding (§4) → run it (§5) → the rules that keep it
running (§6).

## Hard rules

- **The guest never dials Kafka.** No `rdkafka` crate, no `bootstrap.servers` in code.
  `Producer::open(vec![])` / `Consumer::open(config)` return a client the host built from
  the workload's `cosmonic:kafka` binding; whatever that binding sets **wins** over
  anything the guest passes. Put policy in the manifest, never in the code.
- **`handler.topics` and `topics` are different keys.** `handler.topics` is what the host
  subscribes to and dispatches from; `topics` is the grant for the component's OWN
  `producer`/`consumer` calls. Grant exactly what the workload touches — a pure handler
  needs no `topics` at all. Empty or absent `topics` denies everything.
- **Handlers: `handler.group.id` and `dead-letter.topic` are both required.** The deploy
  fails without them. Neither is derived — a derived group would collide across two
  installations of the same manifest, and without a DLQ a permanently failing record
  stalls its partition. Not `group.id`: that key pins a consumer the guest opens itself.
- **Never open a Kafka client per record.** ~100 ms each (DNS + TCP + metadata), measured
  ~1200× the cost of a plain handler dispatch; at high concurrency client churn exhausts
  host fds/threads (`CritSysRes`) and hurts neighbour workloads. Produce from a
  long-lived service, or batch.
- **Async-only WASI p3.** Every function in the package is an `async func`. Rust needs
  wit-bindgen ≥ 0.58 with `async-spawn` + `inter-task-wakeup` on `wasm32-wasip2`. TinyGo
  cannot bind it; there is no Go set (componentize-go is the candidate, not shipped).
- **Malformed input is `permanent`, never a panic.** A trap is retried like a transient
  error; five consecutive traps on one record dead-letter it, and until then that
  partition makes no progress.

## 1. Pick the pattern (four templates, Rust)

Two questions settle it: *does the work start from an HTTP request or from a record*
(request → `http-producer`)? For record-driven work: *does it produce to Kafka, batch, or
keep state across records* (no → `handler-consumer`; yes → `pull-service`)? Reach for
`transactional` only when a replayed or half-applied batch is unacceptable AND every side
effect stays inside Kafka.

| Template id (`rust-kafka-…`) | Shape | Guarantee | Reach for it when the user says | Not this if |
|---|---|---|---|---|
| `http-producer` | HTTP request → `send` / `send-batch` | broker ack per record (`acks` configurable) | "webhook to Kafka", "API that writes to a topic", "ingest gateway", "UI events onto a topic", "produce from a request" | you produce continuously at high rate — open-per-request caps near ~10 req/s per instance; batch (30k+ records/s measured) or hold one producer in a service |
| `handler-consumer` | host pushes a batch of records to `handle` | at-least-once; DLQ on `permanent` | "consume Kafka", "process each record", "filter", "validate", "notify", "sink", "simple consumer" — no cross-record state | the work produces to Kafka (client bootstrap per record, ~1200× a dispatch — use the pull service), needs batching, cross-record state, or your own commit policy |
| `pull-service` | long-running `wasi:cli/run` service owns consumer + producer, batches, commits explicitly | at-least-once; your own DLQ routing | "consume → transform → produce", "enrich a topic", "pipeline", "aggregation window", "batch", "commit offsets myself", "long-lived producer" — **the workhorse** | you need exactly-once (→ transactional), or the work is trivial per record with no produce (handler is less code) |
| `transactional` | pull service + `Transaction`: outputs and input offsets commit atomically | exactly-once (read-process-write) | "exactly-once", "transactional", "payments", "inventory", "ledger", "dedup-sensitive", "read_committed" | throughput matters more than duplicates (a txn round trip per batch), or side effects leave Kafka — a DB write is not covered; then at-least-once + idempotent writes is the honest design |

Per-pattern detail, the shipped manifests and the measured numbers: `references/patterns.md`.

## 2. Know the surface

Four interfaces, one export (`references/cosmonic-kafka.md` has every function and the
`error` record):

| Import | Gives the guest |
|---|---|
| `types` | `consumed-record {topic, partition, offset, key?, value?, headers, timestamp?, timestamp-type, leader-epoch?}`, `produce-record {partition?, key?, value?, headers, timestamp?}`, `produce-ack {partition, offset, timestamp?}`, `partition-ref`, `partition-offset`, `config-entry {key, value}`, `error {code, message, fatal, retriable, txn-requires-abort}` (`code` is every librdkafka `rd_kafka_resp_err_t`, plus `unknown-error-code(s32)`) |
| `producer` | `producer.open(config)`, `send(topic, record) -> produce-ack`, `send-batch(topic, records) -> list<result<ack, error>>` (per-record outcomes), `send-stream(topic, stream)`, `flush`, `partition-count`, `watermark-offsets`; `transaction.begin(&producer)`, `send-offsets(offsets, group-id)`, `commit`, `abort` |
| `consumer` | `consumer.open(config)`, `subscribe(topics)`, `records() -> (stream<consumed-record>, future<result>)`, `commit(offsets)` (`[]` = the stored positions), `committed`, `position`, `seek(partitions, position)`, `pause`/`resume`, `assign`/`incremental-assign`, `rebalances() -> stream<rebalance-event>`, `watermark-offsets`, `offsets-for-times`, `close` |

| Export | The host calls it for | Manifest keys that drive it |
|---|---|---|
| `handler.handle(list<consumed-record>) -> result<option<offset>, handler-error>` | each batch from ONE partition, in offset order — up to `handler.batch.size` (default 100, max 10000) and ~1 MiB | `handler.topics`, `handler.group.id`, `dead-letter.topic`, `handler.batch.size`, `auto.offset.reset` |

Handler return semantics (at-least-once): `Ok(None)` = the whole batch handled, the host
stores past its last record; `Ok(Some(offset))` = handled through that record (its OWN
offset, not the next), the host stores past it and redelivers the rest; `Err(transient)` =
nothing handled, rewind and redeliver indefinitely; `Err(permanent)` = a single-record
batch goes to `dead-letter.topic` and the partition advances — on a longer batch the host
redelivers the FIRST record alone so the verdict lands on one record. **Prefer
`Ok(Some(..))` over an error once any record succeeded**: it keeps the work already done.

## 3. Scaffold and write the code

Scaffold with `cosmonic_project_create(template="rust-kafka-<pattern>", path=…, name=…)`
(or `cosmonic new rust-kafka-<pattern>`); the ids are in `cosmonic_template_list`. Every
scaffold has the same shape:

```
.wash/config.yaml        build command + the cosmonic:kafka binding (workload.hostInterfaces)
                         + workload.environment.config (the service patterns' topics/group)
workload.yaml            the same binding as a Cosmonic Workload (image: built-in registry)
deploy/workload-deployment.yaml   the Kubernetes WorkloadDeployment (Control; prebuilt image)
src/lib.rs               START HERE — the header comment carries the measured numbers
wit/world.wit + wit/deps/cosmonic-kafka-0.3.0/package.wit   vendored; no registry fetch to build
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

Service shape (`wasi:cli/run`; one consumer, ONE long-lived producer, explicit commit):

```rust
let consumer = Consumer::open(vec![cfg("group.id", &group), cfg("auto.offset.reset", "earliest"),
                                   cfg("enable.auto.commit", "false")]).await?;
consumer.subscribe(vec![in_topic]).await?;
let (mut records, _terminal) = consumer.records().await?;
let producer = Producer::open(Vec::new()).await?;      // the binding's client; config merged by the host
let mut pending = Vec::with_capacity(batch);
while let Some(rec) = records.next().await {
    match transform(&rec) { Ok(out) => pending.push(out), Err(why) => { producer.send(dlq.clone(), dead(&rec, why)).await.ok(); } }
    if pending.len() >= batch && producer.send_batch(out_topic.clone(), std::mem::take(&mut pending)).await.is_ok() {
        consumer.commit(Vec::new()).await.ok();          // stored positions, after the outputs are acked
    }
}
```

Exactly-once wraps each batch: `Transaction::begin(&producer)` → `send_batch` →
`txn.send_offsets(one_past_last_per_partition, group_id)` → `txn.commit()`; on any failure
`txn.abort()` and the uncommitted input offsets redeliver the batch. Check
`error.txn_requires_abort` / `error.fatal` (fatal ⇒ reopen the producer; a fresh `begin`
re-fences). Env vars (`IN_TOPIC`, `OUT_TOPIC`, `DLQ_TOPIC`, `GROUP_ID`, `BATCH_SIZE`) come from
`localResources.environment.config` — read them with `std::env::var`, not a generated
`wasi:cli/environment` binding, so every import in the world stays 0.3.0.

Write handlers that are **reuse-safe** (with `poolSize` set, package-level state survives
across batches on a warm instance — a cache, never isolation) and **panic-free**.

## 4. Declare the binding

The scaffold's `.wash/config.yaml` and `workload.yaml` carry the same block; Desktop lays
the config over the world-inferred interfaces for the dev workload and the publish draft:

```yaml
hostInterfaces:
  - namespace: cosmonic
    package: kafka
    version: "0.3.0"
    interfaces: [handler, types]          # EXACTLY what the component imports/exports — over-declaring breaks linking
    config:
      bootstrap.servers: 127.0.0.1:9092   # the entry beats the guest; a guest cannot redirect itself
      broker.address.family: v4           # on Kubernetes: a degraded AAAA path adds seconds per client bootstrap
      handler.topics: "demo.events"       # the subscription (handlers)
      handler.group.id: "demo-events-handler"   # required, stable across redeploys
      dead-letter.topic: "demo.events.dlq"      # required
      auto.offset.reset: earliest
      # topics: "demo.enriched"           # the grant for the component's OWN producer/consumer calls
      # handler.batch.size: "100"         # 1..10000, ≤ ~1 MiB per call
      # transactional.id: "demo-txn-1"    # exactly-once: one STABLE id per logical pipeline (implies enable.idempotence)
    # secretFrom: [kafka-credentials]     # sasl.username / sasl.password / ssl.ca.pem … — never inline
```

Rules the host enforces: the binding entry beats the guest for every key it sets
(`bootstrap.servers`, credentials, the grant, `handler.group.id`, `transactional.id`);
the keys that would hand the *host process* a capability are refused to a workload —
anything loading native code (`plugin.library.paths`, `ssl.engine.location`,
`ssl.providers`), reading a host file by path (`ssl.*.location`, `https.ca.location`,
`sasl.kerberos.keytab`), running a command or reaching a host-chosen URL
(`sasl.kerberos.kinit.cmd`, `sasl.oauthbearer.token.endpoint.url`), or weakening/flooding
the host (`enable.ssl.certificate.verification`, `ssl.endpoint.identification.algorithm`,
`debug`, `statistics.interval.ms`). Credentials themselves are yours: pass TLS material
inline as `ssl.ca.pem` and friends through `config`/`secretFrom`, never as file paths. An
operator can move all of it into the host's own plugin configuration (`hostOwnedKeys`,
`workloadConfig: deny`, named `bindings`); the templates take the self-contained route.
Egress stays `allowedHosts: []` — the plugin dials the broker, the component does not.

## 5. Run it

1. **A broker the host can reach, with the topics the manifest names** — the
   plugin does not create topics. Any Kafka-protocol server on `127.0.0.1:9092` locally
   (`redpanda start --mode dev-container`, or a `kafka-server-start`); then
   `kafka-topics --bootstrap-server 127.0.0.1:9092 --create --topic demo.events` (each
   scaffold's README lists its exact set; the service patterns also need the output and
   DLQ topics).
2. **Build + run:** `cosmonic_dev_start` (the Builder's agent and `cosmonic dev` do the same);
   tail `cosmonic_dev_logs`. On a host WITHOUT the plugin the build succeeds and the start
   fails cleanly with `requires cosmonic:kafka which this host does not provide; not
   restarting (re-apply once the host supports it)` — that is Cosmonic Desktop before
   cosmonic/desktop#389, not a bug in the component: publish, then run it on Cosmonic Control
   (`deploy/workload-deployment.yaml`), or wait for the plugin. Do not retry in a loop.
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
wasm-tools component wit <out>.wasm | grep -E 'cosmonic:kafka/(handler|producer|consumer)@0.3.0'
wasm-tools print <out>.wasm | grep -qE 'async-lift|task-return'
# then make the manifest's `interfaces:` list match those lines exactly
```

## 6. Rules that keep it running (all measured)

- **Scaling** = partitions × replicas, for handlers and pull services alike; a partition
  is only ever assigned to one group member. Handler dispatch runs one loop per assigned
  partition, so `poolSize` decides how many of those concurrent calls land on warm
  instances and `maxConcurrency` how many share one. Do NOT combine `poolSize > 1` with
  `maxConcurrency > 1` on a client-per-request producer (14× regression measured).
- **A slow handler wants `max.poll.interval.ms`, not a smaller pool.** The per-call
  deadline derives from it (the interval less a minute — ten minutes by default) and
  covers the whole batch; the host serves the group's poll timer independently.
- **Rebalancing is `cooperative-sticky`** by default; every member of a group must agree,
  so never mix strategies across deployments sharing a `handler.group.id`.
- **Throughput vs latency in a service** is `BATCH_SIZE`: per-record send+await measured
  ~126 records/s (a broker round trip each); batches of 100+ move tens of thousands per
  second; with low bursty traffic a partial batch waits for the next record — set
  `BATCH_SIZE=1` when end-to-end latency matters more.
- **Never drain an unbounded `records()` stream inside one request-scoped call** — it
  stalls silently past ~10–12k records. Chunk, or move it into a service.
- **Read the `error` record, not just `code`:** `retriable` says retry; `fatal` on a
  producer means reopen it; `txn-requires-abort` means abort the transaction and reprocess.
  `not-leader-for-partition` is "refresh metadata and retry", not "give up".
- **After a broker restart that changed its identity** expect minutes of silent
  no-delivery, then automatic recovery (10 s backoff loop) — alert on consumer lag, send a
  probe record before trusting the pipeline. The symptom catalogue, log line by log line:
  `references/troubleshooting.md`.

## References

- `references/patterns.md` — the four patterns in depth: the shipped manifest of each, the
  env contract of the services, the measured envelope, and the pitfalls per pattern.
- `references/cosmonic-kafka.md` — the driver: every WIT function, the `error` record and
  how to read it, the config keys by owner (driver / workload / host-refused / bounded),
  the built-in limits, and Rust idioms for streams and responses.
- `references/troubleshooting.md` — symptom → cause → fix, including the failures with no
  log line at all.

Related skills: **cosmonic-sandbox** (the loop), **cosmonic-nats** (the same host-owned
shape on NATS — pick NATS when the user wants subjects, JetStream or KV rather than topics).
