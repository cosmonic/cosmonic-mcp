# Kafka troubleshooting on Cosmonic

Symptom → cause → fix, for the messages you will see from the daemon's apply/validate
response, in the host log, in a component's response, or in broker tooling — organised by
symptom, with the failures that produce **no message at all** last, because they look
identical to "still running". The bind-time sentences are the plugin's and Desktop's own
(`cosmonic:kafka@0.5.0`); the runtime symptoms are distilled from the `cosmonic/kafka-demos`
campaign and the `awesome-cosmonic` template notes.

**The rule that prevents most wasted time:** never declare a consumer stalled, done or healthy
from the host log alone. Check the consumer group's **lag** against the topic's high watermark
(`rpk group describe <group>`, `kafka-consumer-groups --describe --group <group>`), and send one
probe record end to end before trusting a pipeline after any disruption.

## The apply or start is refused

On Desktop, its own guard answers at apply — `cosmonic_dev_start`, `cosmonic_workload_apply`
(400 / "rejected") and `cosmonic_workload_validate`, which says exactly what apply would —
for a host-only key, an off-allow-list broker, or two alias spellings of one key in a source.
The runtime's and the plugin's own bind checks (a missing broker, the handler keys, the
grant) run at start and are classified **permanent**: the workload shows Failed with the
sentence, a synchronous PUT answers 400, and nothing retries — fix and re-apply. Messages
name keys, never values.

### `binding (unnamed) of plugin `kafka` resolves without `bootstrap.servers`, which this plugin requires` / `cosmonic:kafka binding `<name>` requires `bootstrap.servers``

**Cause:** the binding names no broker and the host has no `kafka.yaml` default for it. The
plugin declares `bootstrap.servers` required on its binding schema; a guest can never supply
one.

**Fix:** set `bootstrap.servers` in that `cosmonic:kafka` entry (each label needs its own —
the transactional template has two entries), or give the host a default in
`<state_dir>/kafka.yaml` and restart the daemon.

### `hostInterface cosmonic:kafka: `plugin.library.paths` is host-only on Cosmonic Desktop`

**Cause:** the manifest sets one of the keys that would hand the daemon process a capability
(native code, host file paths, commands, TLS-verification switches, `debug`,
`statistics.interval.ms`) — in any spelling, alias or `SECRET_STYLE` included.

**Fix:** drop it. TLS material goes inline as `ssl.ca.pem` / `ssl.certificate.pem` /
`ssl.key.pem` (through `secretFrom`), never as `*.location` paths. An operator who truly
needs the key sets it in the host's `kafka.yaml`.

### `a broker in this binding's `bootstrap.servers` is outside this machine's egress allow-list`

**Cause:** an enterprise machine layer restricts egress and the broker (the manifest's or an
inherited default, port 9092 when none is named) is not on it. A `scheme://` entry never
covers a Kafka broker.

**Fix:** an operator adds the broker to the machine egress allow-list; the workload cannot.

### `component `x`: handler subscription `demo.events` is outside the binding's `topics` grant` / `dead-letter topic `demo.events.dlq` is outside the binding's `topics` grant`

**Cause:** `topics` is the grant for EVERY topic operation on the binding, and absent means
nothing. A handler binding must grant its `handler.topics` subscriptions and its
`dead-letter.topic` (and every produce target when it imports `producer`).

**Fix:** `topics: demo.events,demo.events.dlq` (+ outputs). Never `*` wider than needed.

### `component `x` declares `handler.topics` but no `handler.group.id`` / `… but no `dead-letter.topic``

**Cause:** both are required and never derived (a derived group would be identical across two
installations of the manifest and a shared broker would split the records between them;
without a DLQ a permanently failing record stalls its partition). `consumer.group.id` is a
different key — it names a pull consumer's group — and plain `group.id` is not a binding key.

**Fix:** set both in the handler binding's `config`.

### `components `a` and `b` both export cosmonic:kafka/handler in one workload`

**Cause:** `hostInterfaces` is per workload, so two handler components would join one
`handler.group.id` and split `handler.topics` between them.

**Fix:** one handler component per workload.

### `cosmonic:kafka binding `<name>` grants both `producer` and `transaction`` / `transaction binding `<name>` requires `transactional.id``

**Cause:** a transaction is a separate capability with its own binding, so ordinary sends
cannot join one implicitly; it needs a stable `transactional.id` (and `transaction.group.id`
for `send-offsets`).

**Fix:** two entries — the unnamed `[consumer]` (or `[consumer, producer]`) one and a
`name: transaction` `[transaction]` one — as the transactional template ships.

### `component imports cosmonic:kafka as `transaction`, but the workload binds no cosmonic:kafka interface under that name` / `… but binding `transaction` grants only [consumer]`

**Cause:** the world imports under a label (`import transaction: cosmonic:kafka/…`) and no
entry carries that `name:`, or the entry does not list that interface.

**Fix:** add a `name: <label>` entry whose `interfaces` includes the imported interface. On
Desktop the dev loop and `/v1/synthesize` infer one entry per label from the world; a
project's `.wash/config.yaml` entry of the same `name` replaces it.

### `component imports instance cosmonic:kafka/producer@0.5.0, but a matching implementation was not found in the linker`

**Cause:** the binding's `interfaces` list omits an interface the world imports (the plugin
installs only what is listed), or the world and the binding disagree on the version.

**Fix:** `wasm-tools component wit <out>.wasm`, copy the `cosmonic:kafka/*@0.5.0` lines into
`interfaces` (a labeled import → a `name:` entry). HTTP components otherwise 404 with `no
workload bound to host`; handlers silently never subscribe.

### `requires cosmonic:kafka which this host does not provide; not restarting (re-apply once the host supports it)`

**Cause:** this daemon was built without the `kafka` feature (`--no-default-features`), so
no plugin is registered. Stock Cosmonic Desktop builds carry it (`GET /v1/host` reports a
`kafka` label; `not in this build` is this case).

**Fix:** none in the code. Run it on a stock Desktop build or on Cosmonic Control
(`deploy/workload-deployment.yaml`). Do not retry the start in a loop — the daemon
deliberately does not restart it either.

### The pull line appears, then silence; `WORKLOAD_STATE_NOT_FOUND` on Control

**Cause:** the component was built against WIT whose versions do not match what the host serves
(WIT copied from a fixture rather than fetched from the registry, or a 0.3.0/0.4.0 scaffold).

**Fix:** rebuild with the vendored `wit/deps/cosmonic-kafka-0.5.0/` from the scaffold, or
re-fetch it (`WKG_CONFIG_FILE=./wkg-registries.toml wash wit fetch`); compare `wasm-tools
component wit` against the host's "Host provides interfaces" startup line. If the start line
does appear but readiness flaps right after a host restart, that is the post-restart re-sync —
wait or bounce the workload.

### `failed to retrieve docker credentials … Unable to read config`

**Benign.** Fires on every image pull when there is no docker config present; the pull proceeds
anonymously. If the pull actually failed, the real error follows this line.

## A call fails at runtime

### `topic-authorization-failed` from `send`, `subscribe`, `seek`, `send-offsets` …

**Cause:** the topic is outside the binding's `topics` grant. This is the plugin's check, not
the broker's ACL — it fires even against a broker with no authorization at all.

**Fix:** add the topic to `topics` on THAT binding (a transaction binding needs the output
topic and every input topic whose offsets it enlists).

### `invalid-group-id` from `subscribe`

**Cause:** the consumer binding has no `consumer.group.id`; without it only manual `assign`
works.

**Fix:** set `consumer.group.id` in the binding (not in code — `open()` takes nothing).

### `CritSysResource`: `this workload already has 64 kafka consumer(s) open` / `already owns 64 kafka producer clients` / `already has 32 kafka send-stream(s) draining`

**Cause:** the plugin's per-component ceilings on native clients and streams — typically a
pull consumer opened per invocation and never closed, or unbounded concurrent `send-stream`
calls.

**Fix:** one consumer session per long-lived service, awaited `close()` on the way out; bound
concurrent streams. The message means the plugin's ceiling, not the broker or the OS.

### `state` from `records()`, `rebalances()` or a transaction call

**Cause:** `records()` and `rebalances()` are callable once per consumer; a transaction
resource refuses calls after `commit`/`abort`, and a second `begin` while one is active.

**Fix:** hold the one stream for the loop's lifetime; one transaction at a time per binding.

### `send-offsets requires transaction.group.id on the binding`

**Fix:** set `transaction.group.id` (equal to the consumer binding's `consumer.group.id`) on
the `transaction` entry.

## Producers and consumers are slow

### Every produce or consume takes seconds, with no error

**Symptom:** throughput collapses (13 records/s where you expected thousands), latency in
seconds, nothing in any log; the first operation on a fresh binding-scoped client is slow and
the record's timestamp equals the call time (client bootstrap, not the broker).

**Cause:** a native client bootstrap does a DNS lookup; on a Kubernetes cluster with a
degraded IPv6 (AAAA) path the dual-stack lookup stalls per client.

**Diagnose:** time DNS in the host pod: `kubectl exec <host-pod> -- getent hosts <broker-fqdn>`
vs the same name with a trailing dot; seconds vs milliseconds confirms it. CoreDNS logs show
`i/o timeout` on suffixed queries.

**Fix (any one; all three are hygiene):** `broker.address.family: v4` in the binding (every
shipped manifest carries it); `dnsConfig` `ndots: 1` on the host pod; fix the cluster's DNS
upstream.

### A service crawls at ~100 records/s

**Cause:** `BATCH_SIZE=1` (the template default, chosen for bounded latency) — every record is
a `send` + await, one broker round trip each (~126 records/s measured).

**Fix:** raise `BATCH_SIZE` (capped at 100; no batch timer, so a partial batch waits for the
next record) for a steady stream. Or move the work into a handler, which the host batches.

### Throughput is fine, p99 is terrible

**Cause:** driver trace logging left on (`RUST_LOG=…,plugin_kafka=trace`) raised p99 ~20× in
testing.

**Fix:** turn it off after debugging.

## A partition stops advancing

### `a kafka handler record has failed N consecutive times with no dead-letter.topic configured; this partition will not make progress past it`

**Cause:** a record that deterministically traps or returns `permanent`, with no DLQ. The plugin
retries rather than drops, so the partition is wedged; other partitions keep flowing and the
host is healthy. (Current binds refuse a handler without `dead-letter.topic`; this line is
what a manifest that got past an older check produces.)

**Fix, in order:** add `dead-letter.topic` (inside `topics`) and redeploy (the record is
dead-lettered after 5 attempts and the partition advances — verify the DLQ receives it); or
skip it: stop the workload, `rpk group seek <group> --to <offset+1> --topics <topic>`,
redeploy; or fix the component. Redeploying alone does NOT clear it — the record is
re-served. Prevention: return `permanent` for malformed input, never panic.

### `a kafka handler record has failed too many consecutive times; dead-lettering it`

**Meaning:** five consecutive traps on one record; it went to `dead-letter.topic` with the
original topic/partition/offset as headers and the partition moved on. Read the DLQ, fix the
handler (it should have returned `permanent` on the first attempt).

### The batch keeps being redelivered from the same record

**Cause:** the handler returns `Err(transient)` (or traps) on a record that will never succeed.
Under at-least-once the host rewinds to the batch's first record and redelivers indefinitely
(backing off 100 ms → 30 s).

**Fix:** return `Ok(Some(offset))` for the progress already made, then let the failing record
arrive alone and give it `permanent`. `Ok(Some(..))` after any success is always better than an
error — it keeps the work already done.

### A single call reading a large stream stalls after ~10,000–12,000 records

**Cause:** draining `records()` past that inside one request-scoped invocation stalls silently
at every log level; a fresh invocation reads fine.

**Fix:** never drain an unbounded stream in one request-scoped call — chunk to ≤ 10k per call, or
use a long-lived pull service.

### Slow handler, calls time out

**Cause:** the per-call deadline is `max.poll.interval.ms` less a minute (ten minutes by
default) and covers the WHOLE batch. After a timeout the loop's batch ceiling drops to one and
doubles back with each success, so a handler that cannot finish a full batch settles at a size
it can.

**Fix:** raise `max.poll.interval.ms` (the host serves the group's poll timer independently, so
a long call no longer risks eviction), or lower `handler.batch.size`. Do not shrink the pool —
that does not change the deadline.

## Broker, group and transaction events

### `dropping a kafka rebalance event: the workload is not reading its rebalance stream`

**Meaning:** a rebalance happened that the code did not observe — commonly a request-scoped
invocation that opened a consumer and was abandoned. At-least-once still holds; assume some
duplicates around that moment.

**Fix:** consume from a long-lived service that services its consumer. A service that
deliberately does not drain `rebalances()` will see the depth-32 channel drop under load —
expected.

### Two replicas of a handler get half the records each, or none

**Cause:** the same `handler.group.id` in two deployments that should be independent (they are
one group), or different `partition.assignment.strategy` values in one group (every member must
agree; the default is `cooperative-sticky`).

**Fix:** one group id per logical consumer; never mix strategies across deployments that share
a group.

### Transactional pipeline: members fence each other, or `fatal` on the producer

**Cause:** more than one replica sharing one `transactional.id` (the id is the fencing token;
the interface does not allocate ids per instance), or a fenced/expired producer.

**Fix:** keep `replicas: 1` per `transactional.id`. On `error.fatal` the host retires the
binding's native client — a fresh `begin` re-fences; on `error.txn_requires_abort` abort and
reprocess the batch (the template exits and lets the supervisor restart from committed
offsets).

### Downstream sees the aborted half of a transaction

**Cause:** the downstream consumer reads with the default `isolation.level=read_uncommitted`.

**Fix:** `isolation.level=read_committed` on every consumer of a transactional output topic.

### Pipeline dead for minutes after a broker restart, then recovers on its own

**Cause:** a broker restart that changed its identity (a data-dir wipe) killed the consumer
session; the dispatch loop retries with a 10-second backoff and re-establishes a session — the
measured dead window was 1–8 minutes.

**Guidance:** after maintenance that preserves identity expect fast reconnection; after broker
replacement expect minutes of silent no-delivery, then automatic recovery with no redeploy.
Alert on consumer-group lag, not host logs; send a probe record before trusting the pipeline.

## Benign or expected

- `error serving HTTP client err=hyper::Error(IncompleteMessage)` / `BodyWrite` — an HTTP
  client closed or recycled its connection mid-response; routine pool behaviour, no records
  lost or duplicated. In bulk, ask why responses were slow enough for clients to hang up.
- `MessageSizeTooLarge` / `Broker: Message size too large` — a clean per-record failure; nothing
  partial is written and the other records in the same `send-batch` are unaffected (outcomes are
  per record).
- Committed offsets pause, then resume — normal commit cadence and dispatch pauses. Judge
  completion by lag == 0, never by "offsets stopped moving".
- Nothing dials at daemon boot: the plugin is inert until a workload binds it, so a missing
  broker is a workload-start symptom, never a daemon one.

## Silent failures (no log line)

| Symptom | Cause | Where to look |
|---|---|---|
| Throughput 10–150× low, latency in seconds | degraded cluster DNS on client bootstrap | time DNS in the host pod; `broker.address.family: v4` |
| A call stalls at ~10–12k records | large single-invocation stream drain | chunk, or a pull service |
| A handler never receives anything, no error | the binding's `interfaces` omits `handler`, or the topic has no records past the group's committed offset (`auto.offset.reset: earliest` only applies to a NEW group) | `wasm-tools component wit`; `rpk group describe` |
| Workload retries forever, no error (Control) | WIT version mismatch | rebuild with the vendored/fetched WIT |
| Build succeeds, start says the host does not provide `cosmonic:kafka` | a daemon built without the `kafka` feature | `GET /v1/host` `kafka` label; use a stock build or Control |
