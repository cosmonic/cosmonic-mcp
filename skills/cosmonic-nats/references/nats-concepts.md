# NATS concepts (subjects, core patterns, JetStream, KV, security, server, CLI)

> Adapted from [kaustavdm/nats-skill](https://github.com/kaustavdm/nats-skill) `SKILL.md` (MIT — see `../LICENSE-MIT`). NATS-general reference: the code snippets are `nats.go`, which on Cosmonic Desktop is the **operator and tooling** side (provisioning, the `nats` CLI, server config), never the guest — a component uses the `wasmcloud:nats` WIT surface in `wasmcloud-nats.md` instead. Two Cosmonic-specific corrections to keep in mind while reading: the driver has **no push/pull consumer creation** (provision consumers with the CLI), and a workload never connects — the host does, from `nats.yaml`.

> Docs: https://docs.nats.io | Examples: https://natsbyexample.com

NATS is a subject-based connective layer for distributed systems. Messages route by **subject string**, not hostname:port. Server binary is ~20MB, runs on Raspberry Pi to cloud. CNCF project, Apache 2.0 licensed, 40+ client libraries. Max message payload: 1MB default (configurable up to 64MB; keep under ~8MB in practice).

Two planes — choose before designing:

| Plane | Delivery | Persistence | Use for |
|-------|----------|-------------|---------|
| **Core NATS** | At-most-once | None | Fire-and-forget, RPC, real-time fan-out |
| **JetStream** | At-least/exactly-once | Streams | Durable queues, replay, KV state, work queues |

Deep reference files in this directory:
- [`jetstream.md`](nats-jetstream.md) — full StreamConfig/ConsumerConfig fields, async publish, KV/object-store ops
- [`server-deployment.md`](nats-server.md) — full config options, Docker, Kubernetes, Go embedding struct
- [`security.md`](nats-security.md) — NKey, JWT/operator model, nsc CLI, TLS, auth callout

---

## Subjects

→ https://docs.nats.io/nats-concepts/subjects

```
service.orders.created      # exact (publishers always use exact subjects)
service.orders.*            # * = one token (subscribers only)
service.orders.>            # > = one or more tokens at end (subscribers only)
```

- Dot-separated hierarchy; max 16 tokens, <256 chars recommended
- Alphanumeric, `-`, `_` only (avoid other special chars)
- `$` prefix reserved for system use (`$SYS.*`, `$JS.*`, `$KV.*`, `$SRV.*`)
- Multiple overlapping subs on one connection → duplicate delivery per matching sub

---

## Core NATS Patterns

→ https://docs.nats.io/nats-concepts/core-nats

### Pub/Sub
Fan-out to all subscribers. Zero config — subjects are ephemeral.

```go
nc.Publish("orders.created", data)
nc.Subscribe("orders.*", func(msg *nats.Msg) { /* handle */ })
```

→ https://docs.nats.io/nats-concepts/core-nats/pubsub

### Request/Reply
Requester sends to a subject with a temp reply-to inbox (`_INBOX.<nonce>`). First responder wins.

```go
msg, err := nc.Request("svc.lookup", payload, 2*time.Second)
// err == nats.ErrNoResponders when no subscriber (immediate 503)
```

```go
nc.Subscribe("svc.lookup", func(msg *nats.Msg) { msg.Respond(result) })
```

→ https://docs.nats.io/nats-concepts/core-nats/reqreply

### Queue Groups
Competitive consumers — one random member gets each message. Scale horizontally with zero reconfiguration. Geo-affinity: local consumers served first.

```go
nc.QueueSubscribe("orders.created", "order-processors", handler)
```

→ https://docs.nats.io/nats-concepts/core-nats/queue

---

## JetStream

→ https://docs.nats.io/nats-concepts/jetstream

Enable on server: add `jetstream {}` block or pass `--jetstream` flag.

### Streams

Streams persistently capture Core NATS subjects. Configuration is separate from consumption.

```go
js, _ := nc.JetStream()
js.AddStream(&nats.StreamConfig{
    Name:      "ORDERS",
    Subjects:  []string{"orders.>"},
    Storage:   nats.FileStorage,    // or MemoryStorage
    Replicas:  3,                   // clustered only; 1, 2, 3, or 5
    Retention: nats.LimitsPolicy,   // default
    MaxAge:    24 * time.Hour,
    MaxBytes:  1 << 30,
})
```

**Retention policies** (see [`nats-jetstream.md`](nats-jetstream.md) for decision tree):

| Policy | Behavior |
|--------|----------|
| `LimitsPolicy` | Retain until age/size/count limits hit (default) |
| `WorkQueuePolicy` | Delete on ack; one consumer per subject |
| `InterestPolicy` | Retain while consumers have unread messages |

→ https://docs.nats.io/nats-concepts/jetstream/streams

### Publishing to JetStream

**Always use `js.Publish()`** — not `nc.Publish()` — to receive server ack confirming storage.

```go
ack, err := js.Publish("orders.created", data)
// ack.Stream, ack.Sequence confirm exactly where it was stored

// Exactly-once: include Nats-Msg-Id header (dedup window: 2 min default)
js.PublishMsg(&nats.Msg{
    Subject: "orders.created",
    Header:  nats.Header{"Nats-Msg-Id": []string{uniqueID}},
    Data:    data,
})
```

### Consumers

**Prefer pull consumers for new projects.** Use push only for ordered replay with a single subscriber.

```go
// Durable pull consumer
js.AddConsumer("ORDERS", &nats.ConsumerConfig{
    Durable:       "order-worker",
    FilterSubject: "orders.created",
    AckPolicy:     nats.AckExplicitPolicy,
    DeliverPolicy: nats.DeliverAllPolicy,
    MaxDeliver:    5,
    AckWait:       30 * time.Second,
})

sub, _ := js.PullSubscribe("orders.created", "order-worker")
msgs, _ := sub.Fetch(10, nats.MaxWait(5*time.Second))
for _, msg := range msgs {
    msg.Ack()  // or .Nak(), .InProgress(), .Term()
}
```

```go
// Ordered push consumer (single subscriber, no ack, for replay/inspection)
sub, _ := js.SubscribeSync("orders.>", nats.OrderedConsumer())
```

| Consumer type | When to use |
|--------------|-------------|
| Pull, durable | Scaled workers, batching, explicit flow control |
| Pull, ephemeral | Short-lived processing without persistence |
| Push, ordered | Sequential replay, data inspection (single subscriber) |
| Push, durable | Legacy; avoid for new work |

**Delivery policies:** `DeliverAllPolicy` · `DeliverLastPolicy` · `DeliverLastPerSubjectPolicy` · `DeliverNewPolicy` · `DeliverByStartSequencePolicy` · `DeliverByStartTimePolicy`

**Ack policies:** `AckExplicitPolicy` (default) · `AckNonePolicy` · `AckAllPolicy`

→ https://docs.nats.io/nats-concepts/jetstream/consumers

Full consumer config fields → [`nats-jetstream.md`](nats-jetstream.md)

### Key/Value Store

Built on JetStream streams (prefix `KV_`). Immediately consistent; no read-your-writes guarantee on direct gets (use `Watch` for consistency). Valid key chars: alphanumeric + `_`, `-`, `.`, `=`, `/`.

```go
kv, _ := js.CreateKeyValue(&nats.KeyValueConfig{
    Bucket:  "config",
    TTL:     1 * time.Hour,
    History: 5,     // keep last 5 revisions per key (default: 1)
})

kv.Put("flags.dark-mode", data)
entry, _ := kv.Get("flags.dark-mode")   // entry.Value(), .Revision()
kv.Delete("flags.dark-mode")
kv.Create("lock", data)               // compare-to-null-and-set; fails if exists
kv.Update("lock", newData, revision)  // CAS

watcher, _ := kv.Watch("flags.*")
for entry := range watcher.Updates() { /* nil = end of initial snapshot */ }
```

→ https://docs.nats.io/nats-concepts/jetstream/key-value-store

Full KV API → [`nats-jetstream.md`](nats-jetstream.md)

### Object Store

Chunked file storage on JetStream. Not a distributed filesystem — all objects must fit on the target node.

```go
obs, _ := js.CreateObjectStore(&nats.ObjectStoreConfig{Bucket: "artifacts"})
obs.PutFile("model.bin", "/local/path/model.bin")
obs.GetFile("model.bin", "/dest/model.bin")
obs.Delete("model.bin")
watcher, _ := obs.Watch()
```

→ https://docs.nats.io/nats-concepts/jetstream/obj_store

---

## Connection & Reconnection

→ https://docs.nats.io/using-nats/developer/connecting

```go
nc, err := nats.Connect(
    "nats://s1:4222,nats://s2:4222",     // comma-separated cluster seeds
    nats.UserCredentials("app.creds"),   // JWT+NKey creds file
    // nats.Token("secret")             // or token
    // nats.NkeyOptionFromSeed("user.nk")
    nats.MaxReconnects(-1),             // -1 = infinite (default: 60 attempts)
    nats.ReconnectWait(2*time.Second),
    nats.ReconnectJitter(100*time.Millisecond, time.Second),
    nats.ReconnectBufSize(8<<20),       // 8MB buffer during reconnect
    nats.DisconnectErrHandler(onDisconnect),
    nats.ReconnectHandler(onReconnect),
    nats.ClosedHandler(onClose),
    nats.ErrorHandler(onAsyncError),
)
defer nc.Drain()  // flush pending, then close — never use nc.Close() in production
```

URL schemes: `nats://` (opportunistic TLS) · `tls://` (mandatory TLS) · `ws://` · `wss://`

→ https://docs.nats.io/using-nats/developer/connecting/reconnect

---

## Security

→ https://docs.nats.io/nats-concepts/security

| Method | Complexity | Notes |
|--------|-----------|-------|
| Token | Low | Single shared secret |
| Username/Password | Low | Use bcrypt hashes in server config |
| NKey | Medium | Ed25519; private key never leaves client |
| JWT + NKey | High | Decentralized; new users without server restart |
| TLS client cert | Medium | Cert CN/SAN maps to user |
| Auth callout | High | Delegate auth to an external NATS service |

Subject-level permissions per user:
```conf
users: [{ user: svc, password: "...",
  permissions: { publish: ["events.>"], subscribe: ["_INBOX.>"] } }]
```

Accounts provide isolated namespaces (multi-tenancy). JetStream resources are per-account scoped.

Full NKey, JWT/operator setup, nsc CLI, TLS config → [`nats-security.md`](nats-security.md)

---

## Server Configuration

→ https://docs.nats.io/running-a-nats-service/configuration

```conf
server_name: node-1
port: 4222
http: localhost:8222   # monitoring — bind localhost only; no built-in auth

jetstream {
  store_dir: "/data/nats"    # each cluster node needs its own directory
  max_memory_store: 2GB
  max_file_store: 100GB
}

cluster {
  name: my-cluster
  listen: 0.0.0.0:6222
  routes: ["nats://node-2:6222", "nats://node-3:6222"]
}
```

Hot reload: `nats-server --signal reload`

### JetStream Clustering

→ https://docs.nats.io/running-a-nats-service/configuration/clustering/jetstream_clustering

- Use **3 or 5** JetStream nodes; quorum = n/2 + 1
- Every node needs a unique `server_name` and its own `store_dir`
- Use `server_tags` + stream `Placement` to control replica distribution across zones

### Leaf Nodes

→ https://docs.nats.io/running-a-nats-service/configuration/leafnodes

Lightweight hub-spoke extension. Local clients authenticate locally; traffic bridges to hub only as needed. Best for edge/IoT, air-gapped sites, third-party tenant clusters.

```conf
leafnodes {
  remotes: [{ url: "nats-leaf://hub.example.com:7422",
              credentials: "/etc/nats/edge.creds" }]
}
```

### Embedding in Go

```go
import server "github.com/nats-io/nats-server/v2/server"

opts := &server.Options{
    ServerName: "embedded", Host: "127.0.0.1", Port: 4222,
    JetStream: true, StoreDir: "/tmp/nats",
}
s, _ := server.NewServer(opts)
s.ConfigureLogger()
go s.Start()
if !s.ReadyForConnections(5 * time.Second) { panic("not ready") }
defer s.Shutdown()
nc, _ := nats.Connect(s.ClientURL())
```

Full `server.Options` struct, Docker, Kubernetes → [`nats-server.md`](nats-server.md)

---

## Services API (Micro Framework)

→ https://docs.nats.io/using-nats/developer/services

Build discoverable services over Core NATS — no special server support needed.

```go
import "github.com/nats-io/nats.go/micro"

svc, _ := micro.AddService(nc, micro.Config{Name: "orders", Version: "1.0.0"})
svc.AddEndpoint("create", micro.HandlerFunc(func(req micro.Request) {
    req.Respond(result)
}))
// Endpoints are addressable at "orders.create" by default
```

Auto-handled discovery subjects: `$SRV.PING.>` · `$SRV.INFO.>` · `$SRV.STATS.>`

---

## Subject Mapping & Transforms

→ https://docs.nats.io/nats-concepts/subject_mapping

Applied server-side before routing — transparent to publishers and consumers.

```conf
# Canary deployment: 2% to v2
mappings: {
  "orders.>" : [
    { destination: "orders.v1.{{wildcard(1)}}", weight: 98 }
    { destination: "orders.v2.{{wildcard(1)}}", weight: 2  }
  ]
}

# Deterministic partitioning (preserves per-key order)
mappings: {
  "orders.>" : "orders.shard.{{partition(4,1)}}.{{wildcard(1)}}"
}
```

Transform functions: `{{wildcard(n)}}`, `{{partition(n,idx)}}`, `{{split(n,sep)}}`, `{{SliceFromLeft(n,count)}}`, `{{SliceFromRight(n,count)}}`.

Scopes: root config (default account), per-account, imported subjects, per-stream (via `SubjectTransform` in `StreamConfig`).

---

## Monitoring

→ https://docs.nats.io/running-a-nats-service/nats_admin/monitoring

HTTP on port 8222. **No authentication — bind to localhost only.**

| Endpoint | Returns |
|----------|---------|
| `/varz` | Server state, uptime, memory |
| `/connz` | Connections, throughput, RTT |
| `/jsz` | JetStream streams, consumers, lag |
| `/healthz?js-enabled=1` | Readiness including JetStream |
| `/subsz` | Subscription routing table |
| `/leafz` | Leaf node connections |

Integrations: `nats-top` (live CLI), Prometheus exporter, Grafana dashboards.

---

## NATS CLI

→ https://docs.nats.io/using-nats/nats-tools/nats_cli

```bash
# Context
nats context add prod --server nats://prod:4222 --creds prod.creds
nats context select prod

# Core
nats pub orders.created '{"id":"1"}'
nats sub "orders.>"
nats request svc.lookup '{"q":"foo"}'
nats reply svc.lookup '{"result":"bar"}'

# Streams
nats stream add ORDERS --subjects "orders.>" --storage file --replicas 1
nats stream ls / info ORDERS / rm ORDERS
nats stream purge ORDERS

# Consumers
nats consumer add ORDERS worker --pull --deliver all --ack explicit
nats consumer next ORDERS worker --count 10
nats consumer info ORDERS worker

# KV
nats kv add config && nats kv put config key val
nats kv get config key && nats kv watch config

# Benchmarks & diagnostics
nats bench orders.bench --pub 4 --sub 4 --msgs 1000000
nats server info && nats server ping && nats rtt
```

---

## Common Mistakes

| Mistake | Fix |
|---------|-----|
| `nc.Publish()` to a JetStream subject | Use `js.Publish()` — only way to get storage ack |
| `nc.Close()` on shutdown | Use `nc.Drain()` — flushes buffered messages |
| Push consumer with horizontal scaling | Use pull consumer — push is single-subscriber |
| Two overlapping subscriptions on one conn | Both receive the message — deduplicate intentionally |
| Missing `nc.Flush()` after `Subscribe()` | Subscription is buffered — call `nc.Flush()` before relying on it |
| Multiple `WorkQueuePolicy` consumers per subject | Server enforces one consumer per subject |
| Monitoring port on `0.0.0.0` | No auth exists — bind to `localhost` |
| Ignoring `ErrNoResponders` on `Request()` | Handle 503; no active subscriber on that subject |
| Shared `store_dir` across cluster nodes | Each JetStream node must have its own storage dir |
| MQTT client without JetStream enabled | MQTT requires JetStream for session + retained messages |
