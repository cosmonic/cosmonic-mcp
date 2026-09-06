# The seven `wasmcloud:nats` patterns

Fourteen templates — seven use cases × Rust and Go — compiled into Cosmonic Desktop as
the `rust-nats-<pattern>` / `go-nats-<pattern>` starters (`cosmonic_template_list`,
`cosmonic_project_create`, `cosmonic new`, the Builder's Template menu). Every one
is extracted from a component that ran in the `nats-2.8-testing` campaign (186 measured
cells at 16 KiB, then 1/2/5 MB sizing, both languages), so the guidance is measurement,
not opinion. All seven pass at every size tested once configured; the configuration is
the `cosmonic-nats-tuning` skill.

Common layout of every scaffold: `.wash/config.yaml` (build + the binding under
`workload.hostInterfaces`), `deploy/workload.yaml` (the same binding as a durable
manifest), the handler (`src/lib.rs` / `export_*/handler.go` — START HERE),
`skills/<pattern>/SKILL.md` (dial-in questions, guardrails, pitfalls — read it before
writing code), `docs/tuning.md` (this pattern's envelope), `docs/nats-tuning.md` (the
cross-pattern guide), `scripts/e2e.sh` (drive one message with the local `nats` CLI,
assert the effect), `wit/world.wit` + vendored `wit/deps/wasmcloud-nats-0.1.0/package.wit`.

Local NATS every pattern assumes: `nats-server -js` at `nats://127.0.0.1:4222`
(Settings → Built-in plugins → NATS), plus the objects named below. `scripts/e2e.sh`
additionally expects `nats stream add RECEIPTS --subjects 'done.>' --defaults` where a
pattern emits receipts.

---

## core-subscriber — Core Subscriber

**Reach for it when the user says:** "subscribe to a NATS subject", "consume NATS
messages", "event handler", "message listener", "react to events", "NATS consumer".

A stream of events on a subject, one handler invocation per message. No ack, no
redelivery, no ordering guarantee — the cheapest possible consumer. Telemetry, events,
cache invalidation: a lost message is survivable and latency matters.

**Not this if** losing a message matters: if the handler traps or the subscription
buffer overflows, the message is gone silently.

World: imports `types`, `jetstream`; exports `core-handler`. Shipped config:

```yaml
subject-allow: bench.core,done.core-sink.>
core-subscriptions: bench.core
max-in-flight: "8"          # the memory guard on this path (see tuning §2.1)
```

Dial it in: what is the largest *burst*, not the average rate? `subscription-capacity`
is denominated in messages (stock 1024) and buys `capacity ÷ (arrival − drain)` seconds;
the shed warning prints `would_have_absorbed=N`, the capacity that would have worked.
Each in-flight delivery beyond the warm pool occupies its own fresh instance, so
`max-in-flight × instance footprint` is resident memory — `poolSize: 1` +
`max-in-flight: "8"` measured clean at 1,000 msg/s on a 512 Mi host. Local NATS:
`RECEIPTS` for e2e. Envelope: 50,000 msgs at stock knobs on a hostile 512 Mi rig
**lost 68 %** — which is the point of the JetStream consumer.

## request-reply — Request / Reply

**Reach for it when the user says:** "NATS RPC", "request/reply", "service endpoint",
"answer requests", "query service", "microservice on NATS".

The host delivers the request; the handler publishes the answer to `msg.reply-to`
(core `publish`). Per-request instantiation means it costs nothing idle. The most
robust pattern measured: clean at every size up to 5 MB, at 1, 2 and 3 replicas
(queue-group round-robin, no duplication).

**Not this if** the work outlasts the caller's timeout, or nobody awaits the reply.

World: imports `types`, `core`; exports `core-handler`. Shipped config:

```yaml
subject-allow: svc.echo,_INBOX.>    # the reply subject is an inbox — grant it
core-subscriptions: svc.echo
```

Dial it in: set `poolSize` for latency — Go p50 went 3,190 µs cold → **433 µs** at
10,000 req/s with `poolSize: 8` (Rust measured 458 µs cold; a Rust instance is ~1 MiB).
`max-in-flight` is the real admission limit: each request occupies an instance until it
replies. Core NATS has no error channel — put failure detail in the reply body or
headers; a bare `Err` leaves the caller to time out. At ≥1 MB raise the *caller's*
timeout to 5–10 s. A delivery without `reply-to` is a plain publish on the subject:
return `Ok` and do nothing. Verify: `nats request svc.echo 'hi' --timeout 5s`.

## jetstream-consumer — JetStream Consumer (push)

**Reach for it when the user says:** "durable consumer", "at-least-once", "JetStream
subscriber", "reliable delivery", "don't lose messages", "replay a stream".

JetStream retains, redelivers on failure, and — critically — paces delivery by
acknowledgement, so a slow consumer is throttled instead of overrun. **The safe default
for anything that matters, and the one to start from when unsure.** Every JetStream
cell in the campaign's hostile layer was CLEAN.

**Not this if** you need exactly-once (redelivery is real; be idempotent — the handle's
`sequence()` and `delivery-count()` are your dedup keys) or the lowest latency.

World: imports `types`, `jetstream`; exports `jetstream-handler`. Shipped config:

```yaml
subject-allow: load.>,done.js-sink.>
stream-allow: LOAD
jetstream-subscriptions: LOAD:load.push.>:new:workers   # STREAM:filter:policy:QUEUE GROUP
ack-mode: auto
# max-ack-pending: "16"   max-deliver: "5"   max-in-flight: "8"
# subscription-capacity: "1024"   subscription-capacity-bytes: "33554432"   request-timeout-ms: "10000"
```

Dial it in: **always keep the fourth field** — the queue group makes the consumer
durable (an ephemeral is server-deleted after 120 s idle; delivery stops with no error,
signature `cons=N→0`) and distributes deliveries across replicas. Is the handler
idempotent? Auto-ack (acks on `Ok`, naks on `Err`/trap — right for most handlers) or
manual (nak/term control; forgetting to settle stalls the consumer for the full
ack-wait)? What happens to a message that always fails — set `max-deliver` or `term`
it, or it redelivers forever. Policy field: `new` (default) | `all` | `last` |
`last-per-subject`; an empty slot keeps the default (`STREAM:filter::group`). Local
NATS: `nats stream add LOAD --subjects 'load.>' --defaults` + `RECEIPTS`. At ≥1 MB:
give the NATS server memory first (4 Gi), set the stream's `max_msg_size` near the real
size (the ack window derives from it), `subscription-capacity` 64/32/16 at 1/2/5 MB,
`request-timeout-ms: "10000"`. Changing a config value rebinds the workload and the
previous consumer can keep delivering for up to 120 s — expect duplicates around a
config change.

## jetstream-worker — JetStream Pull Worker

**Reach for it when the user says:** "batch worker", "pull consumer", "process a
backlog", "drain a queue", "paced processing", "fetch messages".

The worker decides when and how much to fetch: `open-pull-consumer(stream, consumer)`
then `fetch(batch, timeout-ms)` / `fetch-with-limits(batch, max-bytes, timeout-ms)`,
settling each handle. Good for expensive per-batch work, rate-limited downstreams, and
large messages — the safest pattern at large payloads. The template is *triggered* by
a core subscription (`pull.run`) and drains the stream in rounds; swap the trigger for
whatever should start a drain.

**Not this if** the path is latency-critical; the fetch round-trip adds delay.

World: imports `types`, `jetstream`; exports `core-handler`. Shipped config:

```yaml
subject-allow: pull.run,done.js-pull.>
stream-allow: LOAD
core-subscriptions: pull.run
```

Dial it in: **`fetch(batch)` materializes `batch × message size` in host memory** —
batch 4 at 1–2 MB, 1–5 at 5 MB (the default 100 at 1 MB is 100 MB per fetch). Drop
fetched handles when done; settling does not release the batch's memory. A fetch can
be refused before anything is delivered (`limit-exceeded`): either over the consumer's
provisioned limits (`info()` reports `max-request-batch` / `max-request-max-bytes` /
`max-waiting` — size against them) or because earlier batches still hold the binding's
memory budget (`info` says nothing; drop the handles). `fetch-stop` tells you why a
batch ended: `batch-filled`, `drained`, `byte-limit`. The pull consumer itself must
exist: `nats consumer add LOAD workers --pull --deliver all --ack explicit --filter
'load.pull.>'`. Local NATS: `LOAD` + `RECEIPTS`.

## kv-store — KV Store Client

**Reach for it when the user says:** "NATS KV", "key/value store", "persist state",
"config store", "compare-and-swap", "durable state".

`kv::open(bucket)` then `get` / `put` / `create` / `update(key, value, expected-revision)`
/ `delete` / `purge` / `keys(filter)` / `history` / `status`. Revisions make CAS work
(`revision-mismatch(current)` carries the current revision, so a retry needs no
re-read); history and a watch channel come free. Configuration, feature flags, session
or device state: the *current value of something*.

**Not this if** you need queries (there are none — key lookups and prefix watches only),
large blobs, or high write rates (each put is a stream publish with an ack).

World: imports `types`, `kv`, `jetstream`; exports `core-handler`. Shipped config:

```yaml
subject-allow: kv.run,done.kv-worker.>
bucket-allow: appkv
core-subscriptions: kv.run
```

Dial it in: `keys(filter)` takes a subject-pattern filter (`>` = every key) and is
capped host-side at 1000 per page — `truncated` distinguishes a partial page from a
complete one; narrow the filter to walk a big bucket. `get` on an absent, deleted or
purged key returns `key-not-found`, never a hang. Set `request-timeout-ms: "10000"`
for values ≥1 MB (the one failure mode above 1 MB is a publish-ack timeout, not
memory); at 5 MB plan for ~1 put/s. Local NATS: `nats kv add appkv` + `RECEIPTS`.

## kv-watcher — KV Watcher

**Reach for it when the user says:** "watch for changes", "config reload", "cache
invalidation", "react to state change", "change data capture".

The host maintains the watch and calls `kv-handler.handle-event(bucket, entry)` on
every put/delete/purge in the bucket or key prefix; `entry` carries key, value,
revision, `created-at-unix-nanos`, `operation`. 100 % watch delivery in every cell
measured, both languages, no tuning required — the quietest pattern in the campaign.

**Not this if** you only need the value now (a `get` is far cheaper), or you need queue
semantics: watch delivery follows KV semantics, and a purge or history-trimmed key can
collapse several logical changes into one event.

World: imports `types`, `kv`, `jetstream`; exports `kv-handler`. Shipped config:

```yaml
subject-allow: done.kv-watch.>
bucket-allow: appkv
kv-watches: appkv:>          # bucket:filter
```

Dial it in: the manifest names bucket and filter under `kv-watches`; if the handler
writes back to the bucket at ≥1 MB, carry `request-timeout-ms: "10000"` like the KV
store does. Local NATS: `appkv` + `RECEIPTS`. Verify: `nats kv put appkv some.key value`.

## fan-out — Fan-Out / Amplifier

**Reach for it when the user says:** "fan out", "broadcast", "scatter", "one-to-many",
"amplify", "notify many subscribers".

One inbound message becomes N outbound units of work (the template republishes each
input 25× onto `fan.work`, or `fanout=N` from the body). Work distribution,
per-recipient notification, scatter-gather. It is a composition: something else
consumes what it emits.

**Not this if** capacity and memory have not been sized to the burst: resident memory
is fan-out × payload (measured ×25 peaks: 860 Mi at 1 MB, 1,360 Mi at 2 MB, 2,676 Mi at
5 MB). A 512 Mi host cannot run ×25 at 1 MB at any setting.

World: imports `types`, `core`; exports `core-handler`. Shipped config:

```yaml
subject-allow: fan.in,fan.work
core-subscriptions: fan.in
max-in-flight: "8"
```

Dial it in: size capacity to the burst — the in-host republish outruns any consumer's
buffer at stock settings. For a 25,000-delivery burst, `max-in-flight: "8192"` +
`subscription-capacity: "65536"` measured clean at 50,000/50,000 on a host sized for it;
on a small host bound `max-in-flight` (8 measured clean at 512 Mi) and let
`subscription-capacity` absorb the burst. **On Cosmonic Desktop keep `max-in-flight`
≤ 1000** — the engine admits 1000 concurrent core instances and deliveries past it
fail rather than queue (tracked as Desktop #413); queue with `subscription-capacity`
instead. Verify: `nats sub fan.work --count=1 &` then `nats pub fan.in hello`.

---

## Cross-cutting facts every pattern repeats

- `subscription-capacity` is the knob that matters, it is denominated in **messages**,
  and its required value spanned **1000×** between guest languages for the same
  workload. Size it to the burst, not the rate.
- **Adding replicas does not add buffer.** Each replica gets its own subscription and
  its own buffer — and without a queue group, its own full copy of the traffic.
- **`poolSize` applies to every delivery path** — request/reply, core, JetStream and
  KV watch. A large latency win at small payloads, a memory cost at ≥1 MB (each warm
  instance retains its heap). Unset or 0 declares state ephemeral: a fresh instance per
  delivery, guaranteed. Pair it with a bounded `max-in-flight`: a warm set does not
  cap concurrency.
- **Verify the export.** A component that builds but exports nothing is a real failure
  mode; every scaffold's CI workflow and `make verify` check for it.
- **Go:** never park on a Go runtime timer in a handler; await the host clock
  (`wasi:clocks/monotonic-clock@0.3.0`, the wasmCloud Go SDK's `sleep` package) if you
  must wait. Duration is fine; *waiting on a timer* traps.
