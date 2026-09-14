# The `cosmonic:kafka@0.3.0` driver

What the guest imports, what the host enforces, and the Rust idioms the templates use. The
WIT is vendored into every scaffold at `wit/deps/cosmonic-kafka-0.3.0/package.wit` — read it
for the full doc comments; this is the working summary. Served by Cosmonic Control's native
`plugin-kafka` (librdkafka in the host process); Cosmonic Desktop gains it with
cosmonic/desktop#389.

## Design, in three sentences

The host owns every Kafka client; a guest gets a `producer` or `consumer` resource, never a
socket. Whatever the workload's `cosmonic:kafka` binding sets **wins** over the config the guest
passes to `open` — a guest's own config is consulted only for keys the entry leaves unset — so
a component cannot redirect itself at another broker or substitute a credential. Every function
is `async func` (a component host plugin cannot serve a sync one), which is why the package
needs wit-bindgen's async support and `wasm32-wasip2`.

## `types`

| Type | Notes |
|---|---|
| `timestamp-ms = s64`, `partition = s32`, `offset = s64` | Negative partitions are invalid; where the partitioner may choose, the field is an `option`, never a magic negative. |
| `position` | `beginning` \| `end` \| `stored` (the group's committed offset, else `auto.offset.reset`) \| `tail(u64)` \| `exact(offset)` — for `seek`/assignment. |
| `header {key, value: option<list<u8>>}` | Keys may repeat; an absent value is distinct from an empty one. |
| `partition-ref {topic, partition}` | What `assign`, `pause`, `resume`, `committed`, `position`, `offsets-for-times` take. |
| `partition-offset {topic, partition, offset, leader-epoch?, metadata?}` | What `commit`, `seek` and `send-offsets` take; `offset` is required. Carry `leader-epoch` through: it is what makes offset fencing work (a commit from a stale leader is rejected). |
| `partition-result {topic, partition, offset?, leader-epoch?, metadata?, error?}` | Per-partition outcome of `commit`/`committed`/`position`/`offsets-for-times`; `error` absent on success — and `no-error` is a success value that appears in these lists. |
| `watermarks {low, high}` | Earliest retained offset; the next offset to be produced. |
| `config-entry {key, value}` | Free-form librdkafka property names verbatim; an unknown/malformed key is `invalid-config` at `open`. |
| `consumed-record` | `topic, partition, offset, key?, value?, headers, timestamp?, timestamp-type (not-available \| create-time \| log-append-time), leader-epoch?`. `value: None` is a tombstone (compacted topics); an empty value is a different thing. |
| `produce-record` | `partition?` (absent → the partitioner), `key?`, `value?` (absent → tombstone), `headers`, `timestamp?` (absent → client/broker stamps it). |
| `produce-ack {partition, offset, timestamp?}` | Where it landed. |
| `error-code` | Every `rd_kafka_resp_err_t` (client-side codes < 0, broker codes ≥ 0) plus `unknown-error-code(s32)` for one this WIT does not name — log it, match on it, treat as non-retriable unless the number says otherwise. Two routinely mishandled: `no-error` is success in per-partition lists; `not-leader-for-partition` means refresh metadata and retry. |
| `error {code, message, fatal, retriable, txn-requires-abort}` | **Branch on the three predicates, not on `code`**: the same code can be retriable in one context and fatal in another, so they are reported per occurrence exactly as `rd_kafka_error_t` does. `fatal` = the client is unusable, reopen it (mainly the idempotent/transactional producer); `retriable` = the identical call may be retried as-is; `txn-requires-abort` = abort before any further transactional work. `message` is not stable — never parse it. |

## `producer`

```wit
resource producer {
  open: static async func(config: list<config-entry>) -> result<producer, error>;
  send: async func(topic: string, %record: produce-record) -> result<produce-ack, error>;
  send-batch: async func(topic: string, records: list<produce-record>) -> result<list<result<produce-ack, error>>, error>;
  send-stream: async func(topic: string, records: stream<produce-record>) -> future<result<_, error>>;
  flush: async func() -> result<_, error>;
  partition-count: async func(topic: string) -> result<u32, error>;
  watermark-offsets: async func(topic: string, partition: partition) -> result<watermarks, error>;
}
resource transaction {
  begin: static async func(producer: borrow<producer>) -> result<transaction, error>;
  send-offsets: async func(offsets: list<partition-offset>, group-id: string) -> result<_, error>;
  commit: async func() -> result<_, error>;
  abort: async func() -> result<_, error>;
}
```

- `open(vec![])` with no config of its own returns the binding's shared client — cheap. An
  `open` WITH config that differs builds a fresh client: DNS + TCP + metadata, ~100 ms,
  roughly three threads plus sockets in the host process. Never per record.
- `send-batch` reports **per-record** outcomes: a `message-size-too-large` on one record leaves
  the others landed. The outer `error` is for the call itself.
- Transactions: `begin` → produce through the SAME producer → `send-offsets` (one past the last
  processed record per input partition, with `leader-epoch`) → `commit`. `abort` on
  `txn-requires-abort`; reopen on `fatal` (a fresh `begin` re-fences). `transactional.id`
  comes from the binding, never from the guest.
- Built-in limits: 64 producers per component; 64 in-flight deliveries per `send-stream`.

## `consumer`

```wit
resource consumer {
  open: static async func(config: list<config-entry>) -> result<consumer, error>;
  subscribe / unsubscribe / subscription
  assign / incremental-assign / incremental-unassign / assignment / assignment-lost
  rebalance-protocol: async func() -> rebalance-protocol;            // none | eager | cooperative
  records: async func() -> result<tuple<stream<consumed-record>, future<result<_, error>>>, error>;
  rebalances: async func() -> result<stream<rebalance-event>, error>; // assign | revoke | lost (list<partition-ref>)
  commit: async func(offsets: list<partition-offset>) -> result<list<partition-result>, error>;
  committed / position / seek / pause / resume / watermark-offsets / offsets-for-times / close
}
```

- `open` takes librdkafka names verbatim (`group.id`, `auto.offset.reset`,
  `enable.auto.commit`, `isolation.level`, …); `group.id` is required for `subscribe`, without
  it only manual `assign` works. Typical errors: `invalid-config`, `invalid-group-id`.
- `subscribe` replaces any previous subscription; a name beginning with `^` is a regex.
  Resolving does not mean partitions are assigned — that arrives on `rebalances` once the
  group settles.
- `records()` returns the stream plus a terminal future (the error that ended it). Drain it
  from a long-lived service; inside one request-scoped call a drain stalls silently past
  ~10–12k records.
- `commit([])` commits the stored positions (the host stores a record's offset when it hands
  it to you). With `enable.auto.commit=false` this is your at-least-once boundary: commit
  only after the batch's outputs are acknowledged.
- Host reads ahead of a pull consumer up to a 64 MiB byte budget (a single record may be the
  whole budget); `enable.auto.offset.store` is forced off.
- `rebalances()` is a depth-32 channel; not draining it is allowed, and under load the host
  logs `dropping a kafka rebalance event` — assume some duplicates around that moment.

## `handler` (the export)

```wit
variant handler-error { transient(option<string>), permanent(option<string>) }
handle: async func(records: list<consumed-record>) -> result<option<offset>, handler-error>;
```

The host subscribes (`handler.topics`, `handler.group.id`), runs one dispatch loop per assigned
partition, and calls `handle` with a never-empty batch from one partition in offset order (≤
`handler.batch.size`, ≤ ~1 MiB). Return value = how far the batch was handled — see SKILL.md §2.
An offset outside the batch is treated as an error (committing past undelivered records would
drop them). The per-call deadline is `max.poll.interval.ms` less a minute (ten minutes by
default) and covers the whole batch; the host serves the group's poll timer independently, so
raising it is the one knob for slow handlers. Five consecutive failures on one record → DLQ
(when `dead-letter.topic` is set) or a stalled partition (when it is not — the deploy refuses
a handler without it).

## Binding config, by owner

The binding is the workload's `cosmonic:kafka` entry under `hostInterfaces`. `interfaces` must
list EXACTLY what the component imports/exports (`[producer, types]`, `[handler, types]`,
`[consumer, producer, types]`) — declaring one it does not use breaks linking of the ones it does.

**Driver-owned keys** (interpreted by the plugin, stripped before librdkafka):

| Key | Meaning |
|---|---|
| `topics` | Comma-separated grant for the component's OWN `producer`/`consumer` calls; `*` unrestricted; empty/absent denies everything. |
| `handler.topics` | What the host subscribes to and dispatches from. Separate from `topics` so a handler that reads one topic and writes another can say so. |
| `handler.group.id` | Required for handlers, never derived. Not `group.id` (that pins a guest-opened consumer). |
| `dead-letter.topic` | Required for handlers. |
| `handler.batch.size` | 1–10000, default 100; also capped around 1 MiB per call. |
| `partition.assignment.strategy` | Default `cooperative-sticky`; every member of a group must agree. |

**Workload-owned connection keys** (the entry beats the guest): `bootstrap.servers` (alias
`metadata.broker.list`), `security.protocol`, `sasl.mechanism` / `sasl.username` /
`sasl.password`, TLS material as PEM (`ssl.ca.pem`, `ssl.key.pem`, `ssl.certificate.pem`),
`acks` (alias `request.required.acks`), `enable.auto.commit`, `compression.type`,
`transactional.id`, `broker.address.family`, `max.poll.interval.ms`, `auto.offset.reset` — any
librdkafka property. Compared by canonical name, so an alias spelling cannot bypass the pin.
Credentials go through `secretFrom`, never inline; `config` → `configFrom` → `secretFrom` merge
in that order.

**Refused to a workload** (they would hand the host process a capability; the host's own
plugin configuration may still set them): `plugin.library.paths`, `ssl.engine.location`,
`ssl.providers`; `ssl.ca.location`, `ssl.key.location`, `ssl.certificate.location`,
`ssl.keystore.location`, `ssl.crl.location`, `https.ca.location`, `sasl.kerberos.keytab`;
`sasl.kerberos.kinit.cmd`, `sasl.oauthbearer.token.endpoint.url`;
`enable.ssl.certificate.verification`, `ssl.endpoint.identification.algorithm`, `debug`,
`statistics.interval.ms`.

**Bounded**: guest-supplied buffer/size keys (`message.max.bytes`, `fetch.message.max.bytes`,
`max.partition.fetch.bytes`, `fetch.max.bytes`, `queue.buffering.max.kbytes`, …) are capped at
100 MiB; `queue.buffering.max.messages=0` is rejected (it disables librdkafka backpressure;
`0x0`, `+0` too).

**Operator takeover.** A host's plugin configuration accepts the same
`config`/`configFrom`/`secretFrom` plus named `bindings` a workload selects by label;
`hostOwnedKeys` claims additional keys for the host and `workloadConfig` (`deny` by default
there) fails a deploy that sets a host-owned key or widens a grant. A platform team puts the
broker and credentials there once; workloads name the label and their topics.

## Component fields that matter

`poolSize` (warm instances between calls), `maxConcurrency` (calls one warm instance serves),
`maxInvocations` (retire after N), `replicas` (WorkloadDeployment). Handler dispatch is one loop
per assigned partition, so `poolSize` and `maxConcurrency` decide how many concurrent calls hit
warm instances vs share one; total throughput stays partitions × replicas. Never combine
`poolSize > 1` with `maxConcurrency > 1` on a client-per-request producer (14× measured
regression). `localResources.allowedHosts: []` — the plugin dials the broker, not the guest.

## Rust idioms (wit-bindgen 0.58, `generate!({ world, generate_all })`)

- **The world name is the pattern's** (`kafka-handler-consumer`, …), not the project's; only
  the crate is renamed by the scaffold. `Cargo.toml`: `wit-bindgen = { version = "0.58.0",
  features = ["macros", "async-spawn", "inter-task-wakeup"] }`, `crate-type = ["cdylib"]`,
  `edition = "2024"`, `cargo build --target wasm32-wasip2 --release`.
- **Records stream**: `let (mut records, _terminal) = consumer.records().await?; while let
  Some(rec) = records.next().await { … }` — keep `_terminal` alive for the loop.
- **HTTP responses** (the producer template): build the body with `wit_stream::new()` +
  `wit_future::new(|| unreachable!())` for trailers, `wit_bindgen::spawn_local` the write,
  `handler::Response::new(Headers::new(), Some(body_rx), trailers_rx)`, `set_status_code`.
- **Config entries**: `ConfigEntry { key: "group.id".into(), value: group.into() }`.
- **Errors**: `format!("{:?}", e.code)` for logs; branch on `e.retriable` / `e.fatal` /
  `e.txn_requires_abort` for behaviour.
- **DLQ record**: copy `key`, `value`, `headers` and push a `Header { key: "x-dlq-reason",
  value: Some(reason.into_bytes()) }`.
- **Env**: `std::env::var("IN_TOPIC")` — the wasip2 target lowers it to `wasi:cli/environment`
  without adding a 0.2.x import to the world.
