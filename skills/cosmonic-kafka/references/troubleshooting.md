# Kafka troubleshooting on Cosmonic

Symptom → cause → fix, for the messages you will see in the host log, in a component's
response, or in broker tooling — organised by symptom, with the failures that produce **no
message at all** last, because they look identical to "still running". Distilled from the
`cosmonic/kafka-demos` campaign (wasmCloud 2.8, `cosmonic:kafka@0.3.0`, Kubernetes + a real
broker) and the `awesome-cosmonic` template notes.

**The rule that prevents most wasted time:** never declare a consumer stalled, done or healthy
from the host log alone. Check the consumer group's **lag** against the topic's high watermark
(`rpk group describe <group>`, `kafka-consumer-groups --describe --group <group>`), and send one
probe record end to end before trusting a pipeline after any disruption.

## The workload will not start

### `requires cosmonic:kafka which this host does not provide; not restarting (re-apply once the host supports it)`

**Cause:** the host has no `cosmonic:kafka` plugin. That is Cosmonic Desktop before
cosmonic/desktop#389. Nothing is wrong with the component or the manifest.

**Fix:** none in the code. Publish (`cosmonic_project_publish`) and run it on Cosmonic Control
with `deploy/workload-deployment.yaml`, or wait for the Desktop plugin. Do not retry the start
in a loop — the daemon deliberately does not restart it either.

### `component imports instance cosmonic:kafka/producer@0.3.0, but a matching implementation was not found in the linker`

**Cause:** the binding's `interfaces` list declares an interface the component does not actually
import (say `consumer` on a produce-only component). The error names an interface the component
DOES import, which points the wrong way.

**Fix:** make `interfaces` exactly match the component's imports/exports — `wasm-tools component
wit <out>.wasm`, copy the `cosmonic:kafka/*` lines verbatim. Do not "grant everything": over-
declaring is what breaks it. HTTP components then 404 with `no workload bound to host`; handlers
silently never subscribe.

### The deploy is refused for a handler

**Cause:** `handler.group.id` or `dead-letter.topic` missing. Both are required and never derived
(a derived group would be identical across two installations of the manifest and a shared
broker would split the records between them; without a DLQ a permanently failing record stalls
its partition). `group.id` is a different key — it pins a consumer the guest opens itself.

**Fix:** set both in the `cosmonic:kafka` entry's `config`.

### The pull line appears, then silence; `WORKLOAD_STATE_NOT_FOUND` on Control

**Cause:** the component was built against WIT whose versions do not match what the host serves
(WIT copied from a fixture rather than fetched from the registry).

**Fix:** rebuild with the vendored `wit/deps/` from the scaffold, or re-fetch it
(`WKG_CONFIG_FILE=./wkg-registries.toml wash wit fetch`); compare `wasm-tools component wit`
against the host's "Host provides interfaces" startup line. If the start line does appear but
readiness flaps right after a host restart, that is the post-restart re-sync — wait or bounce
the workload.

### `failed to retrieve docker credentials … Unable to read config`

**Benign.** Fires on every image pull when there is no docker config present; the pull proceeds
anonymously. If the pull actually failed, the real error follows this line.

## Producers and consumers are slow

### Every produce or consume takes seconds, with no error

**Symptom:** throughput collapses (13 records/s where you expected thousands), latency in
seconds, nothing in any log; with driver trace on, `producer.open` is fast but `send` takes 10 s+
and the record's timestamp equals the call time (client bootstrap, not the broker).

**Cause:** each `open` builds a fresh client, which does a DNS lookup; on a Kubernetes cluster
with a degraded IPv6 (AAAA) path the dual-stack lookup stalls per client.

**Diagnose:** time DNS in the host pod: `kubectl exec <host-pod> -- getent hosts <broker-fqdn>`
vs the same name with a trailing dot; seconds vs milliseconds confirms it. CoreDNS logs show
`i/o timeout` on suffixed queries.

**Fix (any one; all three are hygiene):** `broker.address.family: v4` in the binding (the host
pins it so guests cannot undo it — every shipped manifest carries it); `dnsConfig` `ndots: 1`
on the host pod; fix the cluster's DNS upstream.

### A handler produces one record per dispatch and crawls

**Cause:** opening a `Producer` with its own config inside `handle` pays a full client bootstrap
(~100 ms) per call — measured ~1200× a plain dispatch.

**Fix:** move consume-transform-produce into a pull service (one long-lived producer, batching).
An `open(vec![])` with no config of its own returns the binding's shared client and is cheap,
but the service is still the better shape.

### `open failed: ErrorCode::CritSysRes` (HTTP 500 from a producing component)

**Cause:** concurrent `Producer::open` calls exhausted the host process's threads or file
descriptors — each librdkafka client is ~3 threads plus sockets, and client churn multiplies
that. The host survives but neighbour workloads' opens may fail during the storm.

**Fix:** fewer concurrent opens — lower `maxConcurrency`, reuse a producer in a service, batch.
Keep concurrent opens ≲ 32 per component until the design reuses clients. Raising ulimits is a
stopgap. The message means the HOST ran out, not the broker.

### Throughput is fine, p99 is terrible

**Cause:** driver trace logging left on (`RUST_LOG=…,plugin_kafka=trace`) raised p99 ~20× in
testing.

**Fix:** turn it off after debugging.

## A partition stops advancing

### `a kafka handler record has failed N consecutive times with no dead-letter.topic configured; this partition will not make progress past it`

**Cause:** a record that deterministically traps or returns `permanent`, with no DLQ. The driver
retries rather than drops, so the partition is wedged; other partitions keep flowing and the
host is healthy. (Current deploys refuse a handler without `dead-letter.topic`; this line is what
an older manifest produces.)

**Fix, in order:** add `dead-letter.topic` and redeploy (the record is dead-lettered after 5
attempts and the partition advances — verify the DLQ receives it); or skip it: stop the
workload, `rpk group seek <group> --to <offset+1> --topics <topic>`, redeploy; or fix the
component. Redeploying alone does NOT clear it — the record is re-served. Prevention: return
`permanent` for malformed input, never panic.

### The batch keeps being redelivered from the same record

**Cause:** the handler returns `Err(transient)` (or traps) on a record that will never succeed.
Under at-least-once the host rewinds to the batch's first record and redelivers indefinitely.

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
default) and covers the WHOLE batch.

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

**Cause:** more than one replica sharing one `transactional.id` (the id is the fencing token; the
driver does not yet expose per-instance ids), or a fenced/expired producer.

**Fix:** keep `replicas: 1` per `transactional.id`. On `error.fatal` reopen the producer (a fresh
`begin` re-fences); on `error.txn_requires_abort` abort and reprocess the batch.

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

## Silent failures (no log line)

| Symptom | Cause | Where to look |
|---|---|---|
| Throughput 10–150× low, latency in seconds | degraded cluster DNS on client bootstrap | time DNS in the host pod; `broker.address.family: v4` |
| A call stalls at ~10–12k records | large single-invocation stream drain | chunk, or a pull service |
| Requests fail with `CritSysRes` | host thread/fd exhaustion from client churn | bound concurrent opens; reuse a producer |
| Workload retries forever, no error (Control) | WIT version mismatch | rebuild with the vendored/fetched WIT |
| Build succeeds, start says the host does not provide `cosmonic:kafka` | no plugin on this host (Desktop before #389) | run on Control, or wait for the plugin |
