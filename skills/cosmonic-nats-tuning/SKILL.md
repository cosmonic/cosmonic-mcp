---
name: cosmonic-nats-tuning
description: Size and debug a wasmcloud:nats workload on Cosmonic Desktop from measured numbers, not guesses. Subscription-capacity and max-in-flight against bursts, JetStream ack windows and max-ack-pending, large payloads (1 to 5 MB) and the NATS server profile they need, poolSize and instance reuse, fan-out memory, and the full error catalogue (shedding, backlog full, host memory budget, the OOM signature behind nats IO error storms, DUPLICATES, vanished ephemeral consumers, KV ack timeouts, already-settled, ack-owned-by-host, limit-exceeded fetches, the Go timer trap). Use when a NATS component drops or duplicates messages, stops receiving with no error, reports nats IO error or disconnected, needs more throughput or a bigger payload, or before changing any capacity, ack, timeout, or poolSize setting. Companion to cosmonic-nats (building the component) and cosmonic-sandbox (the deploy loop).
license: Apache-2.0
compatibility: Applies to components on the wasmcloud:nats@0.1.0 driver (Cosmonic Desktop, wasmCloud v2.8+, Cosmonic Control). Numbers were measured in the nats-2.8-testing campaign on a 512 Mi host with NATS 2.12 and JetStream; treat them as calibrated starting points and re-measure on the target host.
metadata:
  version: "1.0.0"
  author: Cosmonic
  upstream: "cosmonic-labs/nats-2.8-testing templates/nats-tuning.md @ 5a437ac"
---

# Cosmonic NATS tuning

How to hit each `wasmcloud:nats` pattern at every payload size, and how to read the
errors on the way. Every number here was measured on a live rig — 186 cells at 16 KiB,
then a dedicated sizing campaign at 1 MB, 2 MB and 5 MB payloads, Rust and Go. The
complete guide, with the derivations and the full error catalogue, is
`references/nats-tuning.md`; this file is the decision surface.

Two facts orient everything else:

1. **Small messages mostly need nothing.** At ≤16 KiB six of the seven patterns run
   clean on stock settings. Only fan-out needs its capacity sized to the burst.
2. **Large messages are a memory problem wearing a networking costume.** At ≥1 MB the
   errors name NATS (`nats: IO error`, `disconnected from NATS`) while the cause is a
   memory limit — the NATS server, the host's guest budget, or in-flight bytes. Size
   memory first, then tune knobs.

## Where each knob lives

| # | Layer | Where it lives |
|---|---|---|
| 1 | **NATS server** — `max_payload`, `max_pending`, `write_deadline`, process memory | server config (`nats.conf`); on Desktop the `nats-server` you run |
| 2 | **JetStream stream** — `max_msg_size`, `max_bytes` | the stream (`nats stream edit`) |
| 3 | **Driver binding** — grants, subscriptions, capacity, ack window, timeouts | the manifest's `hostInterfaces[].config` (and the scaffold's `.wash/config.yaml` `workload.hostInterfaces`) |
| 4 | **Host / component** — guest memory budget, heap, warm pool | Desktop's host settings + `components[]` (`poolSize`, `maxConcurrency`, `maxInvocations`) |

Binding keys a workload may set: `ack-mode`, `request-timeout-ms`, `max-in-flight`,
`subscription-capacity`, `subscription-capacity-bytes`, `max-ack-pending`,
`max-deliver`, `jetstream-subscriptions`, `core-subscriptions`, `kv-watches`. Grants
(`subject-allow`, `stream-allow`, `bucket-allow`) are separate and deny-by-default.
Connection keys are the host's — never in a manifest.

## The three derivations to know by heart

- **Byte budget admits `capacity ÷ payload` messages.** At the 32 MiB
  `subscription-capacity-bytes` default: 2,048 at 16 KiB, **32 at 1 MB, 16 at 2 MB, 6 at
  5 MB**. Above ~1 MB the byte budget, not the message count, binds.
- **The JetStream ack window derives from bytes:**
  `max_ack_pending = min(max-in-flight × 2, subscription-capacity, capacity_bytes ÷ per_message_bytes)`,
  floored at 16, where `per_message_bytes` is the stream's `max_msg_size` if set, else
  the server's `max_payload` (worst case). **Setting the stream's `max_msg_size` near
  the real message size is the single cheapest fix in this guide.**
- **The host-wide backlog ceiling is guest memory ÷ 4**, shared by every subscription on
  the host (128 MiB on a 512 Mi host). Past it deliveries shed with
  `reason="host memory budget"`. Capacity and host memory can only be chosen together.

## By pattern — the one thing each needs

| Pattern | Stock verdict | The setting that matters | At ≥1 MB |
|---|---|---|---|
| core-subscriber | clean at ≤16 KiB; loses under burst | `subscription-capacity` sized to the **burst**, not the rate (`capacity ÷ (arrival − drain)` seconds of protection; the shed warning prints `would_have_absorbed=N` — use it); `max-in-flight` is the memory guard (`poolSize: 1` + `"8"` clean at 1,000 msg/s on 512 Mi) | byte budget binds first; a single subscriber holds ≤6 undelivered 5 MB messages at default. If loss matters, switch to jetstream-consumer |
| request-reply | **clean at every size, 1–3 replicas** | `poolSize` for latency (Go p50 3,190 µs → **433 µs** at 10,000 req/s with `poolSize: 8`; Rust 458 µs cold); `max-in-flight` is the admission limit | raise the **caller's** timeout to 5–10 s; failures go in the reply body — core has no error channel |
| jetstream-consumer | clean (paces by ack) | **queue group in the fourth field** (durable; ephemerals vanish after 120 s idle, signature `cons=N→0`); idempotent handler; `max-deliver` bound | server memory first (4 Gi); stream `max_msg_size`; `subscription-capacity` **64 / 32 / 16** at 1 / 2 / 5 MB; `request-timeout-ms: "10000"` |
| jetstream-worker | clean once batch is sized | **`fetch(batch)` materializes `batch × message size`** — batch 4 at 1–2 MB, 1–5 at 5 MB (default 100 at 1 MB = 100 MB per fetch); drop handles to release memory | the safest pattern at large payloads; `limit-exceeded` = over `info` limits OR earlier handles hold the budget |
| kv-store | clean | `request-timeout-ms: "10000"` for values ≥1 MB (the failure is a publish-ack timeout, not memory); `keys(filter)` ≤1000/page, narrow the filter | ~1 put/s at 5 MB; retry the occasional ack timeout |
| kv-watcher | **100 % delivery every cell, no tuning** | nothing; `kv-watches: bucket:filter` | carry `request-timeout-ms: "10000"` only if the handler writes back at ≥1 MB |
| fan-out | needs sizing even at 16 KiB | capacity to the burst (`max-in-flight: "8192"` + `subscription-capacity: "65536"` clean at 50,000/50,000 on a host sized for it; on a small host bound `max-in-flight` to 8 and let capacity absorb); **on Desktop keep `max-in-flight` ≤ 1000** (engine pool; above it deliveries fail rather than queue) | host memory from the fan-out factor: ×25 peaks **860 Mi at 1 MB, 1,360 Mi at 2 MB, 2,676 Mi at 5 MB** — 2 Gi at 1–2 MB, 4 Gi at 5 MB |

## Sizing rules, in one place

Given payload **M**, burst **B**, fan-out **F**:

```
in-flight bytes         ≈ max_ack_pending × M          (JetStream)
per-subscription buffer ≈ subscription-capacity-bytes  (default 32 MiB)
host backlog ceiling    = guest memory ÷ 4             (shared by ALL subscriptions)
fan-out resident        ≈ F × M                        (size the host from this)
server                  max_payload ≥ M + headers;  max_pending ≥ 10 × M
stream                  max_msg_size ≈ M;  max_bytes ≥ B × M
```

Five rules that held across every measured cell: `max_payload` just above M, never the
64 MiB cap (the driver *reasons* from it); always set the stream's `max_msg_size`; size
host memory from the fan-out factor, not the message size; `poolSize` is a latency win
at small payloads and a memory cost at large ones; never leave the default heap at GiB
scale for components that need MiB.

## The NATS server profile for 1–5 MB

Size the broker **before** touching client knobs — an undersized one reports itself as
client-side errors. One profile covers 1–5 MB:

```
max_payload:    8388608     # 8 MB — NATS's recommended ceiling when raised, NOT 64 MiB
max_pending:   83886080     # 80 MB — ≥ 10× peak message size, or connections stall
write_deadline: "10s"       # large writes need more than the 2 s default
```

```bash
nats stream edit LOAD --max-msg-size=8388608 --force   # ack window derives correctly
nats stream edit LOAD --max-bytes=24GB --force         # ≥ burst × payload, or `discard: old` drops
```

`max_payload` is a config-file setting (`nats-server -c nats.conf`), not a flag. Give
the server 4 Gi at ≥1 MB. At ≥5 MB NATS's own advice is the Object Store pattern
(128 KB chunks + a completion notification); everything here makes 5 MB *work*, but
head-of-line blocking on shared subjects is why it may still be the wrong design.

## Instance reuse (`poolSize`)

Keeps up to N instances warm and applies to **every** delivery path. Small payloads: a
large latency win (Go request/reply 3,190 → 433 µs). Large payloads: a memory cost
(`poolSize × payload` resident) — leave it at 1 above ~1 MB unless memory is sized for
it. Any Go instance needs ~2.3 MiB (Rust ~1 MiB), so the default heap must be ≥4 MiB.
**A warm set does not cap concurrency** — a delivery the pool cannot take spawns a
fresh instance up to `max-in-flight`; pair `poolSize` with a bounded `max-in-flight`.

The state contract: `poolSize` unset/0 declares state ephemeral (fresh instance per
delivery, guaranteed); with it set, package-level state survives across deliveries and
must be treated as a cache — never isolation, persistence, or a place for per-message
secrets. `maxInvocations` retires an instance after N calls; `maxConcurrency` (default
1) bounds a trap's blast radius; a spawned-but-unawaited background task carries
forward on a warm instance.

## Error catalogue — quick index

Every line was produced on a measured run. At large payloads the error text usually
does **not** name the cause: diagnose from the pairing of signatures. Full table with
remediations: `references/nats-tuning.md` §6.

| You see | It means | Do |
|---|---|---|
| `shedding … reason="backlog full" … would_have_absorbed=N` | this subscription's buffer filled | `subscription-capacity ≥ N`; if `queued_bytes ≈ capacity_bytes`, raise `subscription-capacity-bytes`; or raise drain rate (`poolSize`) |
| shed `reason="host memory budget"` | the host-wide ceiling (guest memory ÷ 4) | raise host memory, lower per-subscription capacity, split workloads |
| `derives max-ack-pending 0 … using 16 instead` | per-message estimate is `max_payload` (worst case) | set the stream's `max_msg_size`, or pin `max-ack-pending` |
| storm of `nats: IO error` / `timed out` / `disconnected`, consumers rebuilding, **no shed lines** | a process is being OOM-killed; the connection is collateral | size the NATS server, reduce in-flight bytes, raise host memory |
| `DUPLICATES` (more receipts than sent) | guest cannot settle within ack-wait → redelivery (not loss) | raise drain rate (`poolSize`, `max-in-flight`) or lower delivery; raising `max-ack-pending` alone makes it worse |
| delivery stops, no error, `cons=N→0` | ephemeral push consumer reclaimed after 120 s idle | queue group: `STREAM:filter:new:group` |
| several random-named consumers in `nats consumer ls` | orphaned ephemerals from prior crashes, each receiving everything | delete them, bounce the workload; a queue group avoids the class |
| `needs 2.3MiB of linear memory but --default-heap-memory is 1MiB` | Go heap floor | default heap ≥ 4 MiB |
| deployment refused for a config key | unknown key (fails, with the nearest known key named), a connection key (the host's), or under `deny` a host-owned key / widened grant | fix the key; take grants to the operator / `nats.yaml` |
| `insufficient storage resources available (10047)` | `max-bytes` over `max_storage`, auto-sized from free disk **at startup** | free disk, then restart NATS |
| loss with no driver warning | stream `max_bytes` hit with `discard: old` | `max_bytes ≥ burst × payload` |
| `kv put failed: ack error: timed out` | publish-ack timeout, not memory | `request-timeout-ms: "10000"` |
| `already-settled` | the work was already done | treat as success; only a settle that failed *on the wire* is retryable |
| `ack-owned-by-host` | binding runs `ack-mode: auto` | drop the guest settle or switch to `manual`; `in-progress` works either way |
| pull refused, nothing delivered (`limit-exceeded`) | over the consumer's `info` limits, **or** earlier fetched handles hold the memory budget | size against `info`; drop handles — acking one does not release it |
| `wasm trap: async-lifted export failed to produce a result` (Go) | a Go runtime timer inside a handler | await `wasi:clocks/monotonic-clock@0.3.0` instead; never `time.Sleep`/`After`/`context.WithTimeout` |
| generated Go package has **zero functions**, exit 0 | standalone `wit-bindgen-go` dropped every `async func` | use componentize-go; `make verify` catches the empty component |
| `failed to find export of interface 'wasi:http/incoming-handler'` (Go) | a dependency's `componentize-go.toml` merged its world | pass `-w` explicitly |

Measurement traps if you instrument your own runs: a peak-memory reading of `0` is a
failed read; a peak equal to the limit is censored, not a peak; shedding cells vary
~17 % run to run (verdicts need two runs); receipts read after the fact may have aged
out of the stream.

## References

- `references/nats-tuning.md` — the complete guide: every knob and derivation, per-use-
  case recommendations at 16 KiB / 1 / 2 / 5 MB, the server profile, instance reuse and
  the state contract, and the full error catalogue with conditions and fixes.
- Each scaffold's `docs/tuning.md` — that pattern's measured envelope.
- NATS upstream: [server configuration](https://docs.nats.io/reference/config),
  [sizing & resources](https://docs.nats.io/learn/deployment/sizing-and-resources),
  [slow consumers](https://docs.nats.io/learn/resilient-clients/slow-consumers),
  [Object Store](https://docs.nats.io/learn/object-store/).

Related skills: **cosmonic-nats** (build the component), **cosmonic-sandbox** (deploy it).
