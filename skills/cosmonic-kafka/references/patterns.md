# The four `cosmonic:kafka` patterns

Four templates, Rust, compiled into Cosmonic Desktop as the `rust-kafka-<pattern>` starters
(`cosmonic_template_list`, `cosmonic_project_create`, `cosmonic new`, the Builder's "Kafka
streaming" menu). Every one is vendored from the golden templates in
`cosmonic-labs/awesome-cosmonic` `components/kafka` (@ `f5326286`, the `cosmonic:kafka@0.5.0`
set, tested live against Cosmonic Control 0.11.0 on wasmCloud 2.9). There is no Go set: the
interface is async-only WASI p3, which TinyGo cannot bind, and a template that does not
compile is worse than one that does not exist.

Where a number below is called *measured*, it comes from the `cosmonic/kafka-demos` campaign
on the 0.3.0 driver (a real broker on Kubernetes, single Redpanda, 8 cores) and is kept only
where the mechanism did not change in 0.5.0 — handler dispatch, per-record broker round trips,
DNS on client bootstrap, broker-restart recovery. Everything about opening a producer is gone
with the producer `open` itself.

Common layout of every scaffold: `.wash/config.yaml` (build + the binding under
`workload.hostInterfaces`, plus `workload.environment.config` for the services),
`workload.yaml` (the same binding as a Cosmonic Workload, image = the built-in registry),
`deploy/workload-deployment.yaml` (the Kubernetes `WorkloadDeployment` for Cosmonic Control,
still pointing at the prebuilt upstream image), `src/lib.rs` (START HERE — the header comment
states the pattern's semantics), `wit/world.wit` + vendored `wit/deps/`, `wkg.lock` +
`wkg-registries.toml`.

Broker every pattern assumes: a Kafka-protocol server at `127.0.0.1:9092` with the topics the
manifest names already created — the plugin creates none. Every manifest sets
`broker.address.family: v4` (keeps librdkafka off `::1` when a local broker advertises
`localhost`; on Kubernetes it sidesteps a degraded AAAA path). On Desktop the manifest's
`bootstrap.servers` overrides any `kafka.yaml` default; drop it to inherit one.

---

## handler-consumer — push-mode consumer (start here)

**Reach for it when the user says:** "consume Kafka", "process each record", "event
handler", "filter", "validate", "notify", "sink", "react to events on a topic", and also
"consume → transform → produce", "enrich a topic", "serverless", "scale to zero" — any
elastic per-batch work, with or without output. Least code, hardest to hold wrong: the host
owns the consumer, group membership, offsets, retries and the DLQ, and the guest instances
scale on demand and can return to zero.

**Not this if:** the code needs the consumer *session* — assign, pause, seek, rebalance
events, pull pacing or arbitrary commits (→ pull service) — or outputs and input offsets
must commit in one Kafka transaction (→ transactional).

**Shape.** Exports `cosmonic:kafka/handler@0.5.0` (`handle(list<consumed-record>) ->
result<option<offset>, handler-error>`); the shipped world imports nothing else. The host
calls `handle` with a batch from ONE partition in offset order — up to `handler.batch.size`
(default 100, max 10000) and about 1 MiB. Return `Ok(None)` when the whole batch is handled,
`Ok(Some(offset))` — the last handled record's OWN offset — when one fails part way (the host
keeps that progress and redelivers from the failing record, which then arrives alone and gets
its own verdict), `Err(Transient)` when nothing was handled (rewind, redeliver with a
100 ms → 30 s backoff, indefinitely), `Err(Permanent)` for input that will never succeed (a
single-record batch goes to `dead-letter.topic` with the original topic/partition/offset and
the detail string as headers, and the partition advances). A panic/trap counts as transient,
except that five consecutive traps on the same record dead-letter it. After a failure the
loop's batch ceiling drops to one and doubles back with each success.

**Producing from a handler.** Add `import cosmonic:kafka/producer@0.5.0;` to the world,
`producer` to the binding's `interfaces`, and every output topic to `topics`; then
`producer::send(topic, record).await` / `producer::send_batch(..)`. The host owns and reuses
one native producer for the binding across every instance — no open, no per-call cost beyond
the send. Output is at-least-once (a crash between the send and the offset store replays the
batch); when that is not acceptable, use the transactional service.

**Shipped binding** (`interfaces: [handler]`): `bootstrap.servers`,
`broker.address.family: v4`, `handler.topics: demo.events` (the subscription),
`topics: demo.events,demo.events.dlq` (the grant — it MUST cover the subscription and the
DLQ, or the bind is refused; add the output topics when you add `producer`),
`handler.group.id: demo-events-handler` (required, stable across redeploys — it is what makes
a rolling update resume where the old replicas stopped), `auto.offset.reset: earliest`,
`dead-letter.topic: demo.events.dlq` (required). Component `poolSize: 32`,
`maxConcurrency: 1`; `deploy/`: `replicas: 1`. Topics to create: `demo.events`,
`demo.events.dlq`.

**Scaling.** Per replica, useful concurrency is `min(assigned partitions, 64,
poolSize × maxConcurrency)`. Growing the pool creates no Kafka group member and triggers no
rebalance — the host keeps the native consumer, connection and membership alive while guest
instances come and go (`reclaimWindowSeconds`, `reclaimMinInstances: 0` let the pool return
to zero). Keep `maxConcurrency: 1` unless the handler is safe for overlapping calls on one
instance. `replicas` is different: each is a group member and can move partitions — use it
for availability or more members, not for compute.

**Measured** (0.3.0 campaign; dispatch is unchanged): ceiling > 11,000 records/s (100-byte
records, one Rust component); sustained 300 records/s soak: 6 ms median, 57 ms p99 end to
end, zero loss, flat memory; `replicas` 1 → 2 ≈ 1.6× on one node. Driver trace logging left
on raised p99 ~20× — turn it off after debugging.

**Pitfalls.** Panicking on malformed input (wedges the partition for five attempts) instead of
returning `Permanent`; returning an error after some records succeeded (throws that work away
— return `Ok(Some(offset))`); spending seconds per record without raising
`max.poll.interval.ms` (the per-call deadline is that interval less a minute and covers the
whole batch); leaving the DLQ or an output topic out of `topics`; keeping required state in
the instance (calls land on different instances; idle ones are reclaimed); a second
component exporting `handler` in the same workload (refused — they would split one group).

---

## http-producer — HTTP → Kafka producer

**Reach for it when the user says:** "webhook to Kafka", "API that writes to a topic", "ingest
gateway", "land UI events on a topic", "produce from a request", "request-scoped writes".

**Not this if:** the workload is driven by Kafka records rather than HTTP requests.

**Shape.** Exports `wasi:http/handler@0.3.0`; imports `cosmonic:kafka/producer@0.5.0`.
Routes: `POST /produce?topic=T&key=K&value=V` → one `producer::send`, body
`partition:offset`; `POST /produce-batch?topic=T&count=N&size=S` → 1–10,000 records (values
≤ 1 MiB, ≤ 16 MiB total) via `producer::send_batch`, one line per record (`ok` or the error
code) — outcomes are positional, so one response can carry both. The functions use the
binding's host-owned native client directly: the component creates no Kafka client per
request, and the code never sees a broker address.

**Shipped binding** (`interfaces: [producer]`): `bootstrap.servers`,
`broker.address.family: v4`, `topics: demo.events` (the grant — never `*` wider than needed;
a topic outside it is `topic-authorization-failed`). Plus the `wasi:http` entry with
`interfaces: [handler]` and `host: <name>.localhost`. Component `poolSize: 4`. Topic to
create: `demo.events`.

**Measured** (0.3.0 campaign; a send is still one broker round trip): per-record `send` +
await ~126 records/s per caller; `send-batch` of ~100 → 3,300–5,000 records/s; batches of 500
→ 30,000+. Batch when it is more than a trickle.

**Pitfalls.** Treating a `message-size-too-large` in a batch as a batch failure (outcomes are
per record — the others landed); forgetting that a tombstone is `value: None`, not an empty
value; requesting a topic that is not in the grant (a 500 with `topic-authorization-failed`,
not a broker problem).

---

## pull-service — a long-lived consumer session

**Reach for it when the user says:** "seek", "pause / resume", "assign partitions myself",
"react to rebalances", "commit offsets myself", "custom offset policy", "control the pull
pace", "long-lived session" — the code needs the consumer resource, not just its records.

**Not this if:** it is ordinary elastic event processing, even consume → transform → produce
— the handler is less code, scales without touching group membership and can import the
producer. Or you need exactly-once (→ transactional).

**Shape.** Exports `wasi:cli/run@0.3.0` (a long-running Service; `maxRestarts: 100` on
Control); imports `consumer` and `producer`. One instance owns one `Consumer` session
(`Consumer::open().await` — no arguments; group, offsets policy and auto-commit come from the
binding) and uses the host's binding-scoped producer. Loop: `records()` stream → `transform`
→ buffer → `producer::send_batch` at `BATCH_SIZE` → `commit(vec![])` (the stored positions)
after every output is acknowledged AND every per-partition commit result is clean; a
transform failure sends the record to `DLQ_TOPIC` with an `x-dlq-reason` header (pending
output is flushed first, then that record is committed) — pull mode owns its own
dead-lettering. Any failure exits the service so its supervisor restarts it from committed
offsets. At least once: a crash between produce and commit ⇒ reprocessing, never loss. It
does not scale to zero; adding replicas adds group members and may move partitions.

**Env contract** (`localResources.environment.config`, carried as
`workload.environment.config` in `.wash/config.yaml`): `IN_TOPIC` (default `input`),
`OUT_TOPIC` (`output`), `DLQ_TOPIC` (`input.dlq`), `BATCH_SIZE` (`1`; capped at 100; no batch
timer — a partial batch waits for the next record or the end of the stream). The group is NOT
an env var: it is the binding's `consumer.group.id`. Read with `std::env::var` — not a
generated `wasi:cli/environment` import.

**Shipped binding** (`interfaces: [consumer, producer]`): `bootstrap.servers`,
`broker.address.family: v4`, `topics: demo.events,demo.enriched,demo.events.dlq` — grant
every topic the service touches: input, output AND its DLQ — `consumer.group.id:
demo-pipeline-g1`, `auto.offset.reset: earliest`, `enable.auto.commit: "false"`. Topics to
create: `demo.events`, `demo.enriched`, `demo.events.dlq`.

**Measured** (0.3.0 campaign): `BATCH_SIZE` is the dial — per-record send+await ~126
records/s; batches of 100 move thousands per second. Raise it only for a steady stream where
throughput matters more than partial-batch latency. Scale with partitions × `replicas`.

**Pitfalls.** Committing before the outputs are acked (that is loss, not at-least-once);
draining `records()` inside a request-scoped call instead of a service (stalls silently past
~10–12k records; and `records()` is callable once per consumer); not reading `rebalances()`
and then being surprised by `dropping a kafka rebalance event` (expected if you chose not to
— assume some duplicates around that moment); opening a second consumer per invocation
(each is a bounded group member; 64 per component, then `CritSysResource`); dropping the
resource instead of awaiting `close()` (the group waits out the session timeout).

---

## transactional — exactly-once read-process-write

**Reach for it when the user says:** "exactly-once", "transactional", "payments", "money",
"inventory", "ledger", "dedup-sensitive enrichment", "downstream reads `read_committed`".

**Not this if:** throughput matters more than duplicates (a transaction round trip per
batch), or side effects leave Kafka — a DB write is not covered by the transaction; then
at-least-once + idempotent writes is the honest design.

**Shape.** The pull service plus the separate `transaction` capability, imported under a
label: `import transaction: cosmonic:kafka/transaction@0.5.0;`. Per batch:
`transaction::begin()` → `txn.send_batch(out_topic, outputs)` →
`txn.send_offsets(one past the last processed record per input partition, carrying
`leader-epoch` through for fencing)` → `txn.commit()`. Output records and input offsets
commit atomically; on any failure `txn.abort()`, the service exits, and the supervisor
restarts it from the last atomically committed position; downstream `read_committed` readers
never see the aborted half. The group for `send-offsets` is the binding's
`transaction.group.id` — the guest cannot name one.

**Requirements.** Two bindings on one workload (the plugin refuses `producer` and
`transaction` on the same one): the unnamed `[consumer]` binding with `consumer.group.id`
and `enable.auto.commit: "false"` (offsets travel in the transaction), and a `name:
transaction` `[transaction]` binding with `transactional.id` (host-pinned; implies
`enable.idempotence`; one STABLE id per live instance — it is what fences a restarted
instance) and `transaction.group.id` equal to the consumer's group. Credentials go on both.
Downstream consumers `isolation.level=read_committed`. **Keep `replicas: 1`** per manifest:
every simultaneously live transactional producer needs a distinct stable `transactional.id`,
and the interface does not allocate them per instance.

**Error handling.** On any produce/commit failure check `error.txn_requires_abort` — when set,
`abort` is the only valid terminal operation. When `error.fatal` is set the host retires the
binding's native client; a fresh `begin` re-fences. A concurrent `begin` on the same binding
fails with `state` — one active transaction per binding.

**Env contract:** `IN_TOPIC`, `OUT_TOPIC`, `BATCH_SIZE` (`1`, capped at 100, no timer).

**Shipped bindings:** unnamed `[consumer]` — `bootstrap.servers`, `broker.address.family:
v4`, `topics: demo.events`, `consumer.group.id: demo-txn-g1`, `auto.offset.reset: earliest`,
`enable.auto.commit: "false"`; `name: transaction` `[transaction]` — `bootstrap.servers`,
`broker.address.family: v4`, `topics: demo.events,demo.enriched` (the output topic AND every
input topic whose offsets it enlists), `transactional.id: demo-pipeline-txn-1`,
`transaction.group.id: demo-txn-g1`. Topics to create: `demo.events`, `demo.enriched`.

**Pitfalls.** Scaling replicas with one `transactional.id` (the members fence each other);
granting `producer` and `transaction` on one binding (refused at bind); a `transaction`
binding whose grant misses an input topic (`send-offsets` is checked against it); a side
effect outside Kafka inside the transaction (not covered — it happens on every replay);
measuring throughput and concluding the driver is slow (the round trip is the price of the
guarantee — batch larger).

---

## Operational limits (the plugin's own ceilings)

Native Kafka allocations live outside a component's Wasm memory limit, so the plugin bounds
them; grant only the interfaces a workload needs.

| Resource | Limit |
|---|---|
| Clients per component (`consumer.open()` sessions, binding-scoped producers) | 64 — then `CritSysResource` |
| Handler calls in flight per component | 64, across every assigned partition |
| Handler batch | 100 records by default; `handler.batch.size` 1–10,000; ~1 MiB per call |
| Handler record buffers | 128 MiB shared across partitions |
| Pull-consumer read-ahead | 64 MiB and 512 records |
| Concurrent `send-stream` drains per component | 32 |
| `send-stream` buffer | 64 MiB and 256 records; ~1 MiB per record; 64 in flight |
| Producer queue | 32,768 KiB by default; buffer keys capped at 100 MiB |

## Choosing between them, in one line each

- Starts from an HTTP request → **http-producer** (batch if it is more than a trickle).
- Record in — with or without records out — no need for the session → **handler-consumer**.
- The code must own the consumer session (assign, pause, seek, rebalances, its own commits)
  → **pull-service**.
- A replayed or half-applied batch is unacceptable and everything stays in Kafka →
  **transactional**.
