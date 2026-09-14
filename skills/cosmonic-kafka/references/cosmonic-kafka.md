# The `cosmonic:kafka@0.5.0` driver

What the guest imports, what the host enforces, and the Rust idioms the templates use. The
WIT is vendored into every scaffold at `wit/deps/cosmonic-kafka-0.5.0/package.wit` — read it
for the full doc comments; this is the working summary. Served by Cosmonic Control's native
`plugin-kafka` (librdkafka in the host process), which Cosmonic Desktop compiles into its
daemon behind the default-on `kafka` feature (`daemon/docs/KAFKA.md` in cosmonic/desktop).

## Design, in three sentences

The host owns every Kafka client and the workload's `cosmonic:kafka` **binding is the
capability**: it fixes the broker, credential, group identity, offset policy, client limits
and topic grant before the component runs, and no WIT function accepts a connection or client
property — `producer` is a set of functions on the binding's shared client, `consumer.open()`
and `transaction.begin()` take no arguments. A plain import uses the unnamed binding; a named
import (`import orders: cosmonic:kafka/producer@0.5.0;` — `(implements orders)`) selects the
`name: orders` entry, which is how one workload reaches several clusters, principals or
groups. Every function is `async func` (a component host plugin cannot serve a sync one),
which is why the package needs wit-bindgen's async support and `wasm32-wasip2`.

## `types`

Bound whenever any other `cosmonic:kafka` interface is; it never needs an `interfaces:` entry.

| Type | Notes |
|---|---|
| `timestamp-ms = s64`, `partition = s32`, `offset = s64` | Negative partitions are invalid; where the partitioner may choose, the field is an `option`, never a magic negative. |
| `position` | `beginning` \| `end` \| `stored` (the group's committed offset, else `auto.offset.reset`) \| `tail(u64)` \| `exact(offset)` — for `seek`. |
| `header {key, value: option<list<u8>>}` | Keys may repeat; an absent value is distinct from an empty one. |
| `partition-ref {topic, partition}` | What `assign`, `pause`, `resume`, `committed`, `position`, `offsets-for-times` take. |
| `partition-offset {topic, partition, offset, leader-epoch?, metadata?}` | What `commit`, `seek` and `send-offsets` take; `offset` is required and is the position to RESUME at (one past the last processed record). Carry `leader-epoch` through: it is what makes offset fencing work. |
| `partition-result {topic, partition, offset?, leader-epoch?, metadata?, error?}` | Per-partition outcome of `commit`/`committed`/`position`/`offsets-for-times`; `error` absent on success. |
| `watermarks {low, high}` | Earliest retained offset; the next offset to be produced. |
| `consumed-record` | `topic, partition, offset, key?, value?, headers, timestamp?, timestamp-type (not-available \| create-time \| log-append-time), leader-epoch?`. `value: None` is a tombstone (compacted topics); an empty value is a different thing. |
| `produce-record` | `partition?` (absent → the partitioner), `key?`, `value?` (absent → tombstone), `headers`, `timestamp?` (absent → client/broker stamps it). |
| `produce-ack {partition, offset, timestamp?}` | Where it landed. |
| `error-code` | Every `rd_kafka_resp_err_t` (client-side codes < 0, broker codes ≥ 0) plus `unknown-error-code(s32)` for one this WIT does not name — log it, match on it, treat as non-retriable unless the number says otherwise. Two routinely mishandled: `no-error` is success in per-partition lists; `not-leader-for-partition` means refresh metadata and retry. `topic-authorization-failed` is the grant, not the broker. |
| `error {code, message, fatal, retriable, txn-requires-abort}` | **Branch on the three predicates, not on `code`**: the same code can be retriable in one context and fatal in another, so they are reported per occurrence exactly as `rd_kafka_error_t` does. `fatal` = the native client is unusable — the host rebuilds a binding-owned producer before its next operation; a guest holding a `consumer` must `close` it and `open` a new one. `retriable` = the identical call may be retried as-is; `txn-requires-abort` = abort before any further transactional work. `message` is not stable — never parse it. |

## `producer` — binding-scoped publishing

```wit
interface producer {
  send: async func(topic: string, %record: produce-record) -> result<produce-ack, error>;
  send-batch: async func(topic: string, records: list<produce-record>) -> result<list<result<produce-ack, error>>, error>;
  send-stream: async func(topic: string, records: stream<produce-record>) -> future<result<_, error>>;
  partition-count: async func(topic: string) -> result<u32, error>;
  watermark-offsets: async func(topic: string, partition: partition) -> result<watermarks, error>;
}
```

- **No resource, no `open`, no `close`, no `flush`.** Importing the interface grants one
  producer capability; the host owns and reuses the native producer behind the binding across
  every concurrent instance of the component, and each call has its own completion boundary.
  Rust: `use bindings::cosmonic::kafka::producer; producer::send(topic, record).await`.
- Every topic is checked against the binding's `topics` grant before enqueue.
- `send-batch` reports **per-record** outcomes: a `message-size-too-large` on one record leaves
  the others landed. The outer `error` is for the call itself.
- `send-stream` applies backpressure (the host stops reading while its bounded queues are
  full) and resolves only after every accepted record has an outcome; ~1 MiB per record, 64
  in flight, 32 concurrent drains per component.
- Typical errors: `msg-timed-out` (not acked within `message.timeout.ms`), `queue-full`,
  `unknown-topic-or-part` (topic absent, auto-create off), `not-leader-for-partition` (retry
  after refresh), `topic-authorization-failed` (outside the grant).

## `transaction` — its own binding

```wit
interface transaction {
  resource transaction {
    send / send-batch / send-stream           // as producer, inside this transaction
    send-offsets: async func(offsets: list<partition-offset>) -> result<_, error>;
    commit: async func() -> result<_, error>;
    abort: async func() -> result<_, error>;
  }
  begin: async func() -> result<transaction, error>;
  partition-count / watermark-offsets
}
```

- A separate capability from `producer`, on a separate binding (`interfaces: [transaction]`;
  the plugin refuses a binding that grants both). The binding supplies a stable
  `transactional.id` (required) and, for `send-offsets`, `transaction.group.id` — the guest
  cannot select a group.
- `begin` initialises the binding's transactional client (fencing a previous producer with
  the same `transactional.id`) and leases its single transaction slot; a concurrent `begin`
  fails with `state`. The resource is a strict state machine: produce and `send-offsets`
  while active; exactly one of `commit`, `abort` or drop ends it; after `txn-requires-abort`
  the only valid terminal operation is `abort`. A fatal error retires the native client.
- Every simultaneously live producer needs a distinct stable `transactional.id`; two replicas
  on one id fence each other. The templates ship `replicas: 1`.
- Rust (the template imports it under a label): `use bindings::transaction::{self,
  Transaction}; let txn = transaction::begin().await?; txn.send_batch(..).await?;
  txn.send_offsets(offsets).await?; txn.commit().await?`.

## `consumer` — a session the guest owns

```wit
resource consumer {
  open: static async func() -> result<consumer, error>;          // NO arguments
  subscribe / unsubscribe / subscription
  assign / incremental-assign / incremental-unassign / assignment / assignment-lost
  rebalance-protocol: async func() -> rebalance-protocol;            // none | eager | cooperative
  records: async func() -> result<tuple<stream<consumed-record>, future<result<_, error>>>, error>;
  rebalances: async func() -> result<stream<rebalance-event>, error>; // assign | revoke | lost (list<partition-ref>)
  commit: async func(offsets: list<partition-offset>) -> result<list<partition-result>, error>;
  committed / position / seek / pause / resume / watermark-offsets / offsets-for-times / close
}
```

- `open()` takes nothing: brokers, credentials, group (`consumer.group.id` in the binding,
  translated to the native `group.id`), `auto.offset.reset`, `enable.auto.commit`,
  `isolation.level`, assignment strategy and fetch policy are all fixed by the binding.
  Without `consumer.group.id` only manual `assign` works and `subscribe` fails with
  `invalid-group-id`. Several `open()` calls are several bounded members of the same group;
  64 clients per component, then `CritSysResource`.
- `subscribe` replaces any previous subscription; a name beginning with `^` is a regex, which
  a finite literal `topics` grant refuses. Resolving does not mean partitions are assigned —
  that arrives on `rebalances` once the group settles.
- `records()` is **callable once** (a second call fails with `state`) and returns the stream
  plus a terminal future (the error that ended it). Reading it drives the group heartbeat, so
  read it continuously; drain it from a long-lived service — inside one request-scoped call a
  drain stalls silently past ~10–12k records.
- `commit([])` commits the stored positions (the host stores a record's offset when it hands
  it to you; `enable.auto.offset.store` is forced off). With `enable.auto.commit=false` this
  is your at-least-once boundary: commit only after the batch's outputs are acknowledged, and
  check every `partition-result` for an `error`. Typical errors: `rebalance-in-progress`
  (re-join and retry), `illegal-generation` / `unknown-member-id` (evicted; not yours to
  commit any more).
- Host reads ahead of a pull consumer up to 64 MiB / 512 records.
- `rebalances()` is callable once, depth-32; not draining it is allowed, and under load the
  host logs `dropping a kafka rebalance event` — assume some duplicates around that moment.
- Await `close()` rather than dropping the resource: it triggers an immediate rebalance
  instead of making the group wait out the session timeout.

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
raising it is the one knob for slow handlers. Five consecutive traps on one record → DLQ.
`transient` redeliveries back off from 100 ms to 30 s; after a fatal client error the
dispatch loop rebuilds its consumer every 10 s, forever. A handler may also import `producer`
(host-owned, at-least-once output) — the consume-transform-produce shape.

## Binding config, by owner

The binding is the workload's `cosmonic:kafka` entry under `hostInterfaces` — one per label
(`name:`; the unnamed entry answers a plain import). `interfaces` lists what that binding
grants of `handler`, `producer`, `consumer`, `transaction`; the plugin installs each only if
it is listed, so an unlisted one the world imports is an unsatisfied import at link time, and
a labeled import no binding answers is refused with `component imports cosmonic:kafka as
`<label>`, but the workload binds no cosmonic:kafka interface under that name`. Match the
world exactly; `types` is implied.

**Plugin-owned keys** (interpreted by the plugin, stripped before librdkafka):

| Key | Meaning |
|---|---|
| `topics` | Comma-separated grant for every topic operation on the binding: `handler.topics` subscriptions, the DLQ, produce targets, `subscribe`/`assign`/`seek`/offset and metadata queries, `send-offsets`. `*` = every topic the principal allows; **absent or empty grants nothing** (a handler binding is then refused at bind). A finite grant refuses regex subscriptions. |
| `handler.topics` | What the host subscribes to and dispatches from (comma-separated, literal names). |
| `handler.group.id` | Required for handlers, never derived. Not `consumer.group.id` (a pull consumer's group) and not `group.id` (not a binding key). |
| `dead-letter.topic` | Required for handlers; must be inside `topics`. |
| `handler.batch.size` | 1–10000, default 100; also capped around 1 MiB per call. |
| `consumer.group.id` | The group a `consumer.open()` session joins (translated to native `group.id`; kept distinct so a binding's producer never receives a consumer-only property). Optional — without it only `assign` works. |
| `transaction.group.id` | The group whose offsets a `transaction` binding may enlist with `send-offsets`; required for that call. |
| `partition.assignment.strategy` | librdkafka key the plugin defaults to `cooperative-sticky` for handlers; every member of a group must agree. |

**Workload-owned client keys** (any librdkafka property; the binding is the ONLY source):
`bootstrap.servers` (alias `metadata.broker.list`), `security.protocol`, `sasl.mechanism` /
`sasl.username` / `sasl.password`, TLS material as PEM (`ssl.ca.pem`, `ssl.key.pem`,
`ssl.certificate.pem`), `acks` (alias `request.required.acks`), `enable.auto.commit`,
`auto.offset.reset`, `isolation.level`, `compression.type`, `transactional.id`,
`broker.address.family`, `max.poll.interval.ms`, … Alias pairs are folded to one canonical
spelling wherever keys are compared (Desktop also folds each config SOURCE before merging),
and two spellings of one key set to different values across a binding's entries are refused.
`config` → `configFrom` → `secretFrom` merge in that order. Unknown or malformed properties
fail binding or client initialisation rather than being ignored.

**Refused to a workload** (they would hand the host process a capability; a host's own
`kafka.yaml` may still set them): `plugin.library.paths`, `ssl.engine.location`,
`ssl.providers`; `ssl.ca.location`, `ssl.key.location`, `ssl.certificate.location`,
`ssl.keystore.location`, `ssl.crl.location`, `https.ca.location`, `sasl.kerberos.keytab`,
`sasl.kerberos.principal`, `sasl.oauthbearer.assertion.file`,
`sasl.oauthbearer.assertion.private.key.file`, `sasl.oauthbearer.assertion.jwt.template.file`;
`sasl.kerberos.kinit.cmd`, `sasl.oauthbearer.token.endpoint.url`;
`enable.ssl.certificate.verification`, `ssl.endpoint.identification.algorithm`, `debug`,
`statistics.interval.ms`. Judged on the canonical key, so an alias or a secret-style
spelling of one is refused too.

**Bounded**: buffer/size keys (`message.max.bytes`, `fetch.message.max.bytes`,
`max.partition.fetch.bytes`, `fetch.max.bytes`, `queue.buffering.max.kbytes`,
`queued.min.messages`, …) are capped at 100 MiB; `queue.buffering.max.messages=0` is
rejected (it disables librdkafka backpressure; `0x0`, `+0` too). The producer queue defaults
to 32,768 KiB.

## Credentials and the Desktop host layer

**Control:** `secretFrom` pulls a Kubernetes Secret whose keys are the librdkafka property
names (`sasl.username`, `sasl.password`, `ssl.ca.pem`, …).

**Desktop:** a secret ref's key must be an env-var name, so it cannot be `sasl.password`.
Register the ref with the env-var spelling — `SASL_PASSWORD`, `SASL_USERNAME`,
`SSL_KEY_PASSWORD`, `SASL_OAUTHBEARER_CLIENT_SECRET` — list it under the entry's
`secretFrom`, and the daemon maps the all-caps key to its property (`_` → `.`, lower-cased)
before the client sees it. Values are resolved at start, in memory; no API, event or log
carries one.

**`<state_dir>/kafka.yaml`** (Desktop; read once at boot, edit then restart the daemon) holds
DEFAULTS a binding inherits and the workload's entry overrides key by key:

```yaml
config:                          # the unnamed binding, and the base under every named one
  bootstrap.servers: 127.0.0.1:9092
  broker.address.family: v4
secret_from: [kafka-sasl-password]   # registered refs whose VALUES join that layer at boot
bindings:                        # `name: orders` on a workload's cosmonic:kafka entry
  orders:
    config: { bootstrap.servers: kafka.internal:9093, security.protocol: SASL_SSL, sasl.mechanism: SCRAM-SHA-512 }
    secret_from: [orders-sasl-password]
```

A binding whose broker comes from neither the manifest nor a default is refused (`binding
(unnamed) of plugin `kafka` resolves without `bootstrap.servers`, which this plugin requires`).
Credential VALUES are refused in the file; `secret_from` takes ref names. A root-owned
machine-layer `kafka.yaml` replaces the user's wholesale, and where the machine layer sets an
egress allow-list every broker a binding would dial must be on it (matched like
`allowedHosts`; port 9092 when an entry names none). The daemon runs these checks at apply
(`POST /v1/workloads/validate` says exactly what apply would), at every start on the resolved
config (a refusal there is a permanent failure — the workload shows Failed, nothing retries),
and on the Control-pushed path. Messages name keys, never values. `GET /v1/host` reports a
`kafka` label (`default broker` / `no default broker` / `not in this build`).

## Component fields that matter

`poolSize` (warm instances between calls), `maxConcurrency` (calls one warm instance serves),
`maxInvocations` (retire after N), `reclaimWindowSeconds` / `reclaimMinInstances` (scale the
pool to zero), `replicas` (WorkloadDeployment). Handler dispatch is one loop per assigned
partition, capped at 64 calls in flight per component, so a replica's useful concurrency is
`min(assigned partitions, 64, poolSize × maxConcurrency)`; total throughput stays partitions
× replicas. The templates ship `poolSize: 32`, `maxConcurrency: 1` for the handler and
`poolSize: 4` for the HTTP producer. `localResources.allowedHosts: []` — the plugin dials the
broker, not the guest.

## Rust idioms (wit-bindgen 0.58, `generate!({ world, generate_all })`)

- **The world name is the pattern's** (`kafka-handler-consumer`, `http-kafka-producer`,
  `kafka-pull-service`, `kafka-transactional`), not the project's; only the crate is renamed
  by the scaffold. `Cargo.toml`: `wit-bindgen = { version = "0.58.0", features = ["macros",
  "async-spawn", "inter-task-wakeup"] }`, `crate-type = ["cdylib"]`, `edition = "2024"`,
  `cargo build --target wasm32-wasip2 --release`.
- **Producer**: `use bindings::cosmonic::kafka::producer;` then `producer::send(topic,
  record).await` / `producer::send_batch(topic, records).await` — module functions, no handle.
- **Consumer**: `let consumer = Consumer::open().await?; consumer.subscribe(vec![topic]).await?;
  let (mut records, terminal) = consumer.records().await?; while let Some(rec) =
  records.next().await { … }` — keep `terminal` alive for the loop and await it at the end.
- **Transaction** (labeled import): `use bindings::transaction::{self, Transaction};` —
  `transaction::begin().await?`, then the resource's `send_batch`, `send_offsets`, `commit`,
  `abort`. Offsets: `PartitionOffset { topic, partition, offset: rec.offset + 1, leader_epoch:
  rec.leader_epoch, metadata: None }`, one per input partition.
- **HTTP responses** (the producer template): build the body with `wit_stream::new()` +
  `wit_future::new(|| unreachable!())` for trailers, `wit_bindgen::spawn_local` the write,
  `handler::Response::new(Headers::new(), Some(body_rx), trailers_rx)`, `set_status_code`.
- **Errors**: `format!("{:?}", e.code)` for logs; branch on `e.retriable` / `e.fatal` /
  `e.txn_requires_abort` for behaviour.
- **DLQ record**: copy `key`, `value`, `headers` and push a `Header { key: "x-dlq-reason",
  value: Some(reason.into_bytes()) }`.
- **Env**: `std::env::var("IN_TOPIC")` — the wasip2 target lowers it to `wasi:cli/environment`
  without adding a second `wasi:cli` version to the world.
