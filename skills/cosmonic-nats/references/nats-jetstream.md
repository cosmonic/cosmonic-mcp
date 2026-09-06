# JetStream deep reference (streams, consumers, KV, object store)

> Adapted from [kaustavdm/nats-skill](https://github.com/kaustavdm/nats-skill) (`jetstream.md`, MIT — see `../LICENSE-MIT`). NATS-general reference: the code snippets are `nats.go`, which on Cosmonic Desktop is the **operator and tooling** side (provisioning, the `nats` CLI, server config), never the guest — a component uses the `wasmcloud:nats` WIT surface in `wasmcloud-nats.md` instead.

Source: https://docs.nats.io/nats-concepts/jetstream

## Stream Configuration Fields

```go
type StreamConfig struct {
    Name        string         // Required; no whitespace, dots, wildcards
    Subjects    []string       // Subjects to capture; wildcards OK
    Retention   RetentionPolicy // LimitsPolicy | WorkQueuePolicy | InterestPolicy
    Storage     StorageType    // FileStorage | MemoryStorage
    Replicas    int            // 1-5; clustered deployment required for >1
    MaxAge      time.Duration  // nanoseconds; 0 = unlimited
    MaxBytes    int64          // -1 = unlimited
    MaxMsgs     int64          // -1 = unlimited
    MaxMsgSize  int32          // -1 = unlimited
    MaxMsgsPerSubject int64    // per-subject cap
    Discard     DiscardPolicy  // DiscardOld | DiscardNew
    DiscardNewPerSubject bool  // per-subject new discard (requires DiscardNew)
    NoAck       bool           // disable stream-level ack (use Core NATS publish)
    Duplicates  time.Duration  // dedup window for Nats-Msg-Id (default 2 min)
    Compression StoreCompression // NoCompression | S2Compression
    AllowRollup bool           // enable Nats-Rollup header for compaction
    DenyDelete  bool           // prevent message deletion
    DenyPurge   bool           // prevent stream purge
    AllowDirect bool           // enable direct message get (ADR-31)
    MirrorDirect bool          // enable direct gets on mirrors
    RePublish   *RePublish     // republish stored messages to another subject
    SubjectTransform *SubjectTransformConfig // transform subjects on ingestion
    Sources     []*StreamSource  // aggregate from other streams
    Mirror      *StreamSource    // replicate from exactly one stream (read-only)
    Placement   *Placement       // server tags / cluster name for placement
    Sealed      bool           // no writes allowed (archive state)
}
```

### Mirrors vs Sources

**Mirror**: Read-only replication from exactly one source stream. Clients cannot publish to a mirror. Use for disaster recovery, read replicas, or cross-cluster replication.

**Sources**: Aggregate messages from one or more streams into a single destination stream. The destination is writable. Use for combining regional streams, fan-in patterns, or stream consolidation.

```go
// Mirror (read-only replica of ORDERS)
js.AddStream(&nats.StreamConfig{
    Name:   "ORDERS-MIRROR",
    Mirror: &nats.StreamSource{Name: "ORDERS"},
})

// Source (aggregate from multiple streams)
js.AddStream(&nats.StreamConfig{
    Name: "ALL-EVENTS",
    Sources: []*nats.StreamSource{
        {Name: "ORDERS"},
        {Name: "PAYMENTS"},
    },
})
```

## Consumer Configuration Fields

```go
type ConsumerConfig struct {
    Durable         string          // Named = durable; empty = ephemeral
    Name            string          // ADR-9: alternate consumer name
    Description     string
    FilterSubject   string          // single subject filter (uses {filter} token in perms)
    FilterSubjects  []string        // multiple filters (uses general consumer path in perms — different from FilterSubject)
    DeliverPolicy   DeliverPolicy
    OptStartSeq     uint64          // for DeliverByStartSequencePolicy
    OptStartTime    *time.Time      // for DeliverByStartTimePolicy
    AckPolicy       AckPolicy       // AckExplicit | AckNone | AckAll
    AckWait         time.Duration   // redelivery window (default 30s)
    MaxDeliver      int             // max redeliveries; -1 = unlimited
    Backoff         []time.Duration // progressive retry delays (overrides AckWait)
    ReplayPolicy    ReplayPolicy    // InstantPolicy | OriginalPolicy
    RateLimit       uint64          // bits/sec rate limit for push consumers
    SampleFrequency string          // "100%" for full sampling
    MaxWaiting      int             // max outstanding pull requests
    MaxAckPending   int             // max unacked msgs; -1 = unlimited; default 1000
    FlowControl     bool            // push consumer sliding window flow control
    Heartbeat       time.Duration   // idle heartbeat for push consumers
    HeadersOnly     bool            // deliver headers only, no body
    // Push-specific
    DeliverSubject  string          // subject for push delivery
    DeliverGroup    string          // queue group for push delivery
    // Ordered consumer shorthand (in Go client): nats.OrderedConsumer()
    InactiveThreshold time.Duration // ephemeral auto-delete window
    Replicas        int             // consumer group replicas
    MemoryStorage   bool            // force consumer state to memory
    Metadata        map[string]string
}
```

## JetStream Publish Options (Go)

```go
// Sync publish with options
ack, err := js.Publish("subject", data,
    nats.MsgId("unique-id"),           // for exactly-once dedup
    nats.ExpectStream("ORDERS"),       // fail if subject not in this stream
    nats.ExpectLastMsgId("prev-id"),   // optimistic concurrency check
    nats.ExpectLastSequence(42),       // check sequence
    nats.ExpectLastSubjectSequence(5), // per-subject sequence check
)

// Async publish (batching)
ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
defer cancel()
future, err := js.PublishAsyncMsg(msg)
select {
case ack := <-future.Ok():
    // stored at ack.Sequence in ack.Stream
case err := <-future.Err():
    // handle error
}
// Or wait for all pending
js.PublishAsyncComplete()  // or use context-aware version
```

## Pull Consumer Operations (Go)

```go
sub, _ := js.PullSubscribe("orders.>", "worker",
    nats.Bind("ORDERS", "worker"),    // bind to existing consumer
    nats.ManualAck(),
    nats.AckWait(30*time.Second),
)

// Fetch with timeout
msgs, err := sub.Fetch(100, nats.MaxWait(5*time.Second))

// Fetch with context
msgs, err := sub.FetchBatch(100, nats.Context(ctx))

// Message ack variants
msg.Ack()                              // success, remove from pending
msg.AckSync()                          // ack and wait for server confirmation
msg.Nak()                              // failure, redeliver soon
msg.NakWithDelay(5 * time.Second)      // redeliver after delay
msg.InProgress()                       // extend ack wait (keep processing)
msg.Term()                             // terminal failure, stop redelivery
msg.TermWithReason("invalid payload")  // terminal failure with reason

// JetStream message metadata
meta, _ := msg.Metadata()
// meta.Sequence.Stream  — position in stream
// meta.Sequence.Consumer — delivery count
// meta.NumDelivered — how many times this msg was delivered
// meta.NumPending — messages remaining in consumer
// meta.Timestamp — original publish time
// meta.Stream, meta.Consumer — names
```

## Stream Operations (Go)

```go
js, _ := nc.JetStream()

// CRUD
info, err := js.StreamInfo("ORDERS")
err = js.DeleteStream("ORDERS")
err = js.PurgeStream("ORDERS")
err = js.PurgeStream("ORDERS", &nats.StreamPurgeRequest{
    Subject: "orders.failed",
    Keep:    0,
    Sequence: 100,
})

// Update stream config
err = js.UpdateStream(&nats.StreamConfig{...})

// List streams
for info := range js.Streams() { ... }
for name := range js.StreamNames() { ... }

// Direct get (requires AllowDirect: true on stream)
msg, err := js.GetMsg("ORDERS", 42)          // by sequence
msg, err := js.GetLastMsg("ORDERS", "orders.123")  // last for subject
```

## Retention Policy Decision Tree

```
Need at-most-once delivery?
  → Core NATS (no JetStream)

Need persistence, multiple consumers, replay?
  → LimitsPolicy (default)

Need work queue (each message processed exactly once, delete on ack)?
  → WorkQueuePolicy
  → Only 1 pull consumer per subject allowed
  → Use pull consumers for horizontal scaling

Need messages only while subscribers are active?
  → InterestPolicy
  → Messages deleted after all consumers ack
  → Good for transactional patterns
```

## Consumer Type Decision Tree

```
Single sequential consumer, inspection, replay?
  → Ordered push consumer (nats.OrderedConsumer())

Horizontally scaled workers / batching / explicit flow control?
  → Pull consumer (recommended for new projects)

Legacy, real-time push to subject (single subscriber)?
  → Push consumer with DeliverSubject
  → Add DeliverGroup for queue-group delivery to multiple instances
```

## KV Store — Full Operation Reference

Valid key characters: alphanumeric plus `_`, `-`, `.`, `=`, `/`. Keys support dot-separated hierarchies for wildcard watches (`flags.*`).

```go
// Create/bind bucket
kv, err := js.CreateKeyValue(&nats.KeyValueConfig{
    Bucket:       "config",
    Description:  "Application configuration",
    TTL:          24 * time.Hour,
    History:      10,            // keep last 10 revisions (default: 1)
    MaxValueSize: 1 << 20,       // 1MB per value
    MaxBytes:     1 << 30,       // 1GB bucket total
    Storage:      nats.FileStorage,
    Replicas:     3,
})
kv, err := js.KeyValue("config")  // bind to existing

// Basic CRUD
revision, err := kv.Put("key", value)       // create or update
revision, err := kv.Create("key", value)    // fails if exists (compare-to-null-and-set)
revision, err := kv.Update("key", value, lastRevision)  // CAS; fails if revision mismatch
entry, err := kv.Get("key")
err = kv.Delete("key")     // adds delete marker (visible in history)
err = kv.Purge("key")      // removes all history including delete markers

// entry fields
entry.Key()
entry.Value()
entry.Revision()   // monotonic counter
entry.Created()    // time.Time
entry.Delta()      // pending ops since last check
entry.Operation()  // KeyValuePut | KeyValueDelete | KeyValuePurge

// Keys listing (only keys with non-deleted values)
keys, err := kv.Keys()
keys, err := kv.Keys(nats.Context(ctx))

// Watching
watcher, err := kv.Watch("feature.*")
watcher, err := kv.WatchAll()
watcher, err := kv.Watch("key",
    nats.IncludeHistory(),     // get full history, not just current
    nats.IgnoreDeletes(),      // skip delete/purge operations
    nats.MetaOnly(),           // headers only, no values
)
for entry := range watcher.Updates() {
    if entry == nil { break }  // nil signals end of initial values
    // process entry
}
watcher.Stop()

// Bucket info and management
status, err := kv.Status()
// status.Bucket(), status.Bytes(), status.Values(), status.History(), status.TTL()
err = js.DeleteKeyValue("config")
```

## Object Store — Full Operation Reference

```go
// Create/bind bucket
obs, err := js.CreateObjectStore(&nats.ObjectStoreConfig{
    Bucket:      "artifacts",
    Description: "Build artifacts",
    TTL:         7 * 24 * time.Hour,
    MaxChunkSize: 128 * 1024,    // chunk size for large files (default: 128KB)
    Storage:     nats.FileStorage,
    Replicas:    3,
    MaxBytes:    10 << 30,       // 10GB
})
obs, err := js.ObjectStore("artifacts")

// Store and retrieve
info, err := obs.PutFile("model.bin", "/local/path/model.bin")
info, err := obs.Put(&nats.ObjectMeta{Name: "config.json"}, reader)
err = obs.GetFile("model.bin", "/dest/model.bin")
result, err := obs.Get("config.json")  // returns io.ReadCloser

// Metadata
info, err := obs.GetInfo("model.bin")
// info.Name, info.Description, info.Headers, info.Size, info.Chunks, info.Digest

// Management
err = obs.Delete("old-artifact")
infos, err := obs.List()
watcher, err := obs.Watch()
for info := range watcher.Updates() { ... }

// Cleanup
err = js.DeleteObjectStore("artifacts")
```

## Stream Subject Transforms (Server Config)

Applied on ingestion, before storage. Original subject stored; transformed subject used for routing to consumers.

```conf
# In stream config (server-side, via NATS CLI or API)
# nats stream add with --subject-transform flag, or via API:
{
  "name": "ORDERS",
  "subjects": ["orders.v1.>"],
  "subject_transform": {
    "src": "orders.v1.>",
    "dest": "orders.{{wildcard(1)}}"
  }
}
```

## JetStream API Subjects (Internal)

All JetStream operations are Core NATS request/reply on `$JS.API.*`:

```
$JS.API.STREAM.CREATE.<name>
$JS.API.STREAM.UPDATE.<name>
$JS.API.STREAM.DELETE.<name>
$JS.API.STREAM.INFO.<name>
$JS.API.STREAM.LIST
$JS.API.STREAM.NAMES
$JS.API.CONSUMER.CREATE.<stream>
$JS.API.CONSUMER.DELETE.<stream>.<consumer>
$JS.API.CONSUMER.INFO.<stream>.<consumer>
$JS.API.CONSUMER.MSG.NEXT.<stream>.<consumer>  # pull request
$JS.API.MSG.GET.<stream>
$JS.API.MSG.DELETE.<stream>
$JS.API.DIRECT.GET.<stream>  # direct get (ADR-31)
```

This means JetStream is fully accessible via Core NATS pub/sub in any client.
