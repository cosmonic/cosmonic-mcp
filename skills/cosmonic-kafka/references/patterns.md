# The four `cosmonic:kafka` patterns

Four templates, Rust, compiled into Cosmonic Desktop as the `rust-kafka-<pattern>` starters
(`cosmonic_template_list`, `cosmonic_project_create`, `cosmonic new`, the Builder's "Kafka
streaming" menu). Every one is vendored from the golden templates in
`cosmonic-labs/awesome-cosmonic` `components/kafka` (@ `77d3153`), and every design guideline
and number below was measured against a real broker on Kubernetes (wasmCloud 2.8,
`cosmonic:kafka@0.3.0`, single Redpanda broker, 8 cores) — shape, not ceiling. There is no
Go set: the interface is async-only WASI p3, which TinyGo cannot bind, and a template that
does not compile is worse than one that does not exist.

Common layout of every scaffold: `.wash/config.yaml` (build + the binding under
`workload.hostInterfaces`, plus `workload.environment.config` for the services),
`workload.yaml` (the same binding as a Cosmonic Workload, image = the built-in registry),
`deploy/workload-deployment.yaml` (the Kubernetes `WorkloadDeployment` for Cosmonic Control,
still pointing at the prebuilt upstream image), `src/lib.rs` (START HERE — the header comment
carries the measured numbers), `wit/world.wit` + vendored `wit/deps/`, `wkg.lock` +
`wkg-registries.toml`.

Broker every pattern assumes: a Kafka-protocol server at `127.0.0.1:9092` with the topics the
manifest names already created — the plugin creates none. Kubernetes manifests add
`broker.address.family: v4` and point at `my-kafka.kafka.svc.cluster.local:9092`.

---

## http-producer — HTTP → Kafka producer

**Reach for it when the user says:** "webhook to Kafka", "API that writes to a topic", "ingest
gateway", "land UI events on a topic", "produce from a request", "request-scoped writes".

**Not this if:** you produce continuously at high rate. An open-per-request producer caps
near ~10 req/s per instance (each `open` builds a client: DNS + TCP + metadata, ~100 ms).
Batch (`send-batch`, 30k+ records/s measured at 500 per batch), or hold one producer in a
pull service.

**Shape.** Exports `wasi:http/handler@0.3.0`; imports `cosmonic:kafka/{types,producer}`.
Routes: `POST /produce?topic=T&key=K&value=V` → one `send`, body `partition:offset`;
`POST /produce-batch?topic=T&count=N&size=S` → N records via `send-batch`, one line per record
(`ok` or the error code). `Producer::open(Vec::new())` returns the binding's client — the
host merges `bootstrap.servers` and the rest in, the code never sees a broker address.

**Shipped binding** (`interfaces: [producer, types]`): `bootstrap.servers`,
`broker.address.family: v4`, `topics: "demo.events"` (the grant — never `*` wider than
needed). Plus the `wasi:http` entry with `host: <name>.localhost`. Component: `poolSize: 4`,
`maxConcurrency` left at 1.

**Measured.** Per-record `send` + await ~126 records/s; `send-batch` of ~100 → 3,300–5,000
records/s; HTTP `send-batch` of 500 → 30,000+. `poolSize 8` + `maxConcurrency 32` on this
client-per-request shape → ~23 req/s against ~300 for either knob alone (14× regression,
deterministic). 128-way concurrency → `open failed: ErrorCode::CritSysRes` (host thread/fd
exhaustion from client churn) — keep concurrent opens ≲ 32 per component until the design
reuses a producer.

**Pitfalls.** Combining `poolSize > 1` with `maxConcurrency > 1`; treating a `MessageSizeTooLarge`
in a batch as a batch failure (outcomes are per record — the others landed); forgetting that a
tombstone is `value: None`, not an empty value.

---

## handler-consumer — push-mode consumer

**Reach for it when the user says:** "consume Kafka", "process each record", "filter",
"validate", "notify", "sink", "simple consumer", "react to events on a topic" — per-record work
with no cross-record state. Least code, hardest to hold wrong: the host owns the consumer,
offsets, retries and the DLQ.

**Not this if:** the work per record produces to Kafka (a client bootstrap per record measured
~1200× a plain dispatch — use the pull service), needs batching, cross-record state, or your own
commit policy.

**Shape.** Exports `cosmonic:kafka/handler@0.3.0` (`handle(list<consumed-record>) ->
result<option<offset>, handler-error>`); imports `types` only. The host calls `handle` with a
batch from ONE partition in offset order — up to `handler.batch.size` (default 100, max 10000)
and about 1 MiB. Return `Ok(None)` when the whole batch is handled, `Ok(Some(offset))` — the
last handled record's OWN offset — when one fails part way (the host keeps that progress and
redelivers from the failing record, which then arrives alone and gets its own verdict),
`Err(Transient)` when nothing was handled (rewind, redeliver indefinitely), `Err(Permanent)`
for input that will never succeed (a single-record batch goes to `dead-letter.topic` and the
partition advances). A panic/trap counts as transient, except that five consecutive traps on
the same record dead-letter it.

**Shipped binding** (`interfaces: [handler, types]`): `bootstrap.servers`,
`broker.address.family: v4`, `handler.topics: "demo.events"` (the subscription),
`handler.group.id: "demo-events-handler"` (required, stable across redeploys — it is what
makes a rolling update resume where the old replicas stopped), `auto.offset.reset: earliest`,
`dead-letter.topic: "demo.events.dlq"` (required). No `topics` — a pure handler needs no grant;
add the output topic to `topics` if you add `producer` to `interfaces`. Component `poolSize: 4`.

**Measured.** Dispatch ceiling > 11,000 records/s (100-byte records, one Rust component);
sustained 300 records/s soak: 6 ms median, 57 ms p99 end to end, zero loss, flat memory;
`replicas` 1 → 2 ≈ 1.6× on one node (CPU-bound beyond; a multi-node cluster scales further).
Driver trace logging left on raised p99 ~20× — turn it off after debugging.

**Pitfalls.** Panicking on malformed input (wedges the partition for five attempts) instead of
returning `Permanent`; returning an error after some records succeeded (throws that work away
— return `Ok(Some(offset))`); spending seconds per record without raising
`max.poll.interval.ms` (the per-call deadline is that interval less a minute and covers the
whole batch); opening a `Producer` inside `handle` — an `open` with no config of its own returns
the binding's shared client and is cheap, but consume-transform-produce is still the pull
service's job.

---

## pull-service — the workhorse

**Reach for it when the user says:** "consume → transform → produce", "enrich a topic",
"pipeline", "aggregation window", "batch", "commit offsets myself", "custom offset policy",
"long-lived producer".

**Not this if:** you need exactly-once (→ transactional), or the work is trivial per record with
no produce (the handler is less code).

**Shape.** Exports `wasi:cli/run@0.3.0` (a long-running Service, `maxRestarts` on Control);
imports `types`, `consumer`, `producer`. One instance owns one `Consumer` (`group.id`,
`auto.offset.reset`, `enable.auto.commit=false`) and ONE long-lived `Producer`. Loop:
`records()` stream → `transform` → buffer → `send_batch` at `BATCH_SIZE` → `commit(vec![])`
(the stored positions) after the outputs are acknowledged; failures of your transform go to
YOUR DLQ topic with an `x-dlq-reason` header — pull mode owns its own dead-lettering. At
least once: a crash between produce and commit ⇒ reprocessing, never loss.

**Env contract** (`localResources.environment.config`, carried as
`workload.environment.config` in `.wash/config.yaml`): `IN_TOPIC` (default `input`),
`OUT_TOPIC` (`output`), `DLQ_TOPIC` (`input.dlq`), `GROUP_ID` (`pull-service-g1`),
`BATCH_SIZE` (`100`). Read with `std::env::var` — not a generated `wasi:cli/environment`
import, so every import in the world stays at 0.3.0.

**Shipped binding** (`interfaces: [consumer, producer, types]`): `bootstrap.servers`,
`broker.address.family: v4`, `topics: "demo.events,demo.enriched,demo.events.dlq"` — grant
every topic the service touches: input, output AND its DLQ.

**Measured.** `BATCH_SIZE` is the dial: per-record send+await ~126 records/s; batches of 100+
move tens of thousands per second; with low bursty traffic a partial batch waits for the next
record — `BATCH_SIZE=1` when end-to-end latency matters more than throughput. Scale with
partitions × `replicas`.

**Pitfalls.** Committing before the outputs are acked (that is loss, not at-least-once);
draining `records()` inside a request-scoped call instead of a service (stalls silently past
~10–12k records); not draining `rebalances()` and then being surprised by
`dropping a kafka rebalance event` (expected if you chose not to — assume some duplicates
around the moment); a Go-style GC dropping the consumer mid-run does not apply in Rust —
ownership keeps the resource alive for the loop.

---

## transactional — exactly-once read-process-write

**Reach for it when the user says:** "exactly-once", "transactional", "payments", "money",
"inventory", "ledger", "dedup-sensitive enrichment", "downstream reads `read_committed`".

**Not this if:** throughput matters more than duplicates (a transaction round trip per
batch), or side effects leave Kafka — a DB write is not covered by the transaction; then
at-least-once + idempotent writes is the honest design.

**Shape.** The pull service plus `Transaction`. Per batch: `Transaction::begin(&producer)` →
`send_batch` the outputs → `txn.send_offsets(one past the last processed record per input
partition, carrying `leader-epoch` through for fencing, group_id)` → `txn.commit()`. Output
records and input offsets commit atomically; on any failure `txn.abort()` and the uncommitted
input offsets redeliver the batch; downstream `read_committed` readers never see the aborted
half.

**Requirements.** `transactional.id` in the binding (host-pinned; implies
`enable.idempotence`) — one STABLE id per logical pipeline, it is what fences a restarted
instance; consumer `enable.auto.commit=false` (offsets travel in the transaction); downstream
consumers `isolation.level=read_committed`. **Keep `replicas: 1`**: more than one member
needs a distinct `transactional.id` per member, which the driver does not yet expose
per-instance.

**Error handling.** On any produce/commit failure check `error.txn_requires_abort` — when set,
`abort` and reprocess. When `error.fatal` is set the producer is finished: reopen it (a fresh
`begin` re-fences).

**Env contract:** `IN_TOPIC`, `OUT_TOPIC`, `GROUP_ID` (`txn-pipeline-g1`), `BATCH_SIZE`.

**Shipped binding** (`interfaces: [consumer, producer, types]`): `bootstrap.servers`,
`broker.address.family: v4`, `topics: "demo.events,demo.enriched"`,
`transactional.id: "demo-pipeline-txn-1"`.

**Pitfalls.** Scaling replicas with one `transactional.id` (the members fence each other);
a side effect outside Kafka inside the transaction (not covered — it happens on every
replay); measuring throughput and concluding the driver is slow (the round trip is the
price of the guarantee — batch larger).

---

## Choosing between them, in one line each

- Starts from an HTTP request → **http-producer** (batch if it is more than a trickle).
- Record in, no record out, no state between records → **handler-consumer**.
- Record in, record out, or batching/state/commit policy → **pull-service**.
- A replayed or half-applied batch is unacceptable and everything stays in Kafka →
  **transactional**.
