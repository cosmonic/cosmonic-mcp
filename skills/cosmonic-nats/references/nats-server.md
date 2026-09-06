# NATS server deployment reference (config, clustering, leaf nodes, embedding)

> Adapted from [kaustavdm/nats-skill](https://github.com/kaustavdm/nats-skill) (`server-deployment.md`, MIT — see `../LICENSE-MIT`). NATS-general reference: the code snippets are `nats.go`, which on Cosmonic Desktop is the **operator and tooling** side (provisioning, the `nats` CLI, server config), never the guest — a component uses the `wasmcloud:nats` WIT surface in `wasmcloud-nats.md` instead.

Sources:
- https://docs.nats.io/running-a-nats-service/configuration
- https://docs.nats.io/running-a-nats-service/configuration/clustering
- https://docs.nats.io/running-a-nats-service/configuration/clustering/jetstream_clustering
- https://docs.nats.io/running-a-nats-service/configuration/leafnodes
- https://pkg.go.dev/github.com/nats-io/nats-server/v2/server

## Server Installation

```bash
# macOS
brew install nats-server

# Linux (download binary — replace version as needed)
# Check latest at: https://github.com/nats-io/nats-server/releases
NATS_VER=v2.11.3
curl -L "https://github.com/nats-io/nats-server/releases/download/${NATS_VER}/nats-server-${NATS_VER}-linux-amd64.tar.gz" | tar xz
sudo mv nats-server-${NATS_VER}-linux-amd64/nats-server /usr/local/bin/

# Docker
docker run -p 4222:4222 -p 8222:8222 nats:latest

# Kubernetes (Helm)
helm repo add nats https://nats-io.github.io/k8s/helm/charts/
helm install nats nats/nats
```

## Server Configuration Format

NATS config uses a flexible format:
- Comments: `#` or `//`
- Assignment: `=`, `:`, or whitespace
- Values: primitives, lists `[...]`, maps `{...}`
- Numbers with units: `1K` (1000), `1KB` (1024), `1G`, `1GB`
- Variables: `$VAR_NAME` (block-scoped or environment)
- Includes: `include "other.conf"`

## Minimal Server Configs

### Standalone (dev/test)
```conf
port: 4222
jetstream {}
http: localhost:8222
```

### Standalone with JetStream and Auth
```conf
server_name: nats-1
port: 4222
http: localhost:8222

jetstream {
  store_dir: "/data/nats"
  max_memory_store: 2GB
  max_file_store: 100GB
}

accounts {
  APP: {
    jetstream: enabled
    users: [
      { user: app, password: "$2a$11$..." }  # bcrypt hash
    ]
  }
}
no_auth_user: app
```

### 3-Node JetStream Cluster

All three nodes need identical `cluster.name`. Each needs unique `server_name` and its own `store_dir`.

**node-1.conf:**
```conf
server_name: node-1
port: 4222

jetstream {
  store_dir: "/data/nats/node-1"
}

cluster {
  name: my-cluster
  listen: 0.0.0.0:6222
  routes: [
    "nats://node-2.internal:6222"
    "nats://node-3.internal:6222"
  ]
}
```

**node-2.conf / node-3.conf:** Same structure, change `server_name` and route list.

Start: `nats-server -c node-1.conf`

### Leaf Node (edge/IoT)

```conf
server_name: edge-1
port: 4222

jetstream {
  store_dir: "/data/nats"
}

leafnodes {
  remotes: [{
    url: "nats-leaf://hub.example.com:7422"
    credentials: "/etc/nats/edge.creds"
  }]
}
```

Hub server must have `leafnodes { listen: "0.0.0.0:7422" }`.

## Important Server Ports

| Port | Protocol | Purpose |
|------|----------|---------|
| 4222 | TCP | Client connections |
| 6222 | TCP | Cluster routes |
| 7422 | TCP | Leaf node connections |
| 7522 | TCP | Gateway (supercluster) |
| 8222 | HTTP | Monitoring (no auth — bind localhost!) |

## Configuration Key Limits

| Setting | Default | Max |
|---------|---------|-----|
| `max_payload` | 1MB | 64MB |
| `max_connections` | 64K | — |
| `max_pending` | 64MB | — |
| `max_control_line` | 4KB | — |
| `ping_interval` | 2 min | — |
| `write_deadline` | 10s | — |

## JetStream Server Settings

```conf
jetstream {
  store_dir: "/data/nats"     # REQUIRED: unique per server node
  domain: "hub"               # optional: domain name for JetStream
  max_memory_store: 2GB       # default: 75% of RAM
  max_file_store: 100GB       # default: 1TB
  cipher: chacha              # or "aes" — encryption at rest
  key: "$JETSTREAM_KEY"       # encryption key (from env)
}
```

## Authentication Configurations

### Token
```conf
authorization {
  token: "my-secret-token"
}
```

### Username/Password
```conf
authorization {
  username: app
  password: "secret"
}
```

### Multiple Users
```conf
authorization {
  users: [
    { user: publisher, password: "p1",
      permissions: { publish: ["events.>"], subscribe: [] } }
    { user: consumer, password: "p2",
      permissions: { publish: [], subscribe: ["events.>"] } }
    { user: admin, password: "p3" }
  ]
}
```

### NKey
```conf
authorization {
  users: [
    { nkey: "UABC123..." }  # public NKey
  ]
}
```

### JWT/Operator (decentralized)
```conf
operator: "/etc/nats/operator.jwt"
resolver: {
  type: full
  dir: "/data/nats/jwt"
  allow_delete: false
  interval: "2m"
}
resolver_preload: {
  ACCABC123: "/etc/nats/app-account.jwt"
}
```

## Signal Management

```bash
nats-server --signal reload       # reload config
nats-server --signal stop         # graceful stop
nats-server --signal quit         # immediate stop
nats-server --signal reopen       # reopen log files (log rotation)
nats-server --signal ldm          # lame duck mode (drain connections)
```

## Monitoring Endpoints

All return JSON. Bind monitoring port to localhost only.

```bash
curl http://localhost:8222/varz    # server variables and stats
curl http://localhost:8222/connz   # connections (paginated)
curl http://localhost:8222/jsz     # JetStream stats
curl http://localhost:8222/jsz?streams=1&consumers=1  # include stream/consumer details
curl http://localhost:8222/healthz           # basic health
curl http://localhost:8222/healthz?js-enabled=1       # require JetStream healthy
curl http://localhost:8222/routez  # cluster routes
curl http://localhost:8222/leafz   # leaf node connections
curl http://localhost:8222/gatewayz # gateway connections
curl http://localhost:8222/subsz   # subscription topology
curl http://localhost:8222/accountz # accounts
```

Useful query params for `/connz`:
- `?subs=1` — include subscription info
- `?sort=bytes_to` — sort by bytes sent
- `?limit=50&offset=0` — pagination

## Embedding NATS Server in Go

```go
package main

import (
    "time"
    server "github.com/nats-io/nats-server/v2/server"
    "github.com/nats-io/nats.go"
)

// Load config from file instead of programmatic Options:
//   opts, err := server.ProcessConfigFile("/etc/nats/server.conf")
//   opts = server.MergeOptions(opts, overrides)

func startEmbedded() (*server.Server, *nats.Conn, error) {
    opts := &server.Options{
        ServerName: "embedded",
        Host:       "127.0.0.1",
        Port:       4222,
        JetStream:  true,
        StoreDir:   "/tmp/nats-embedded",
        // Clustering:
        // Cluster: server.ClusterOpts{
        //     Name:  "my-cluster",
        //     Host:  "0.0.0.0",
        //     Port:  6222,
        // },
        // Logging:
        // Debug: false,
        // Trace: false,
        // LogFile: "/var/log/nats.log",
    }

    s, err := server.NewServer(opts)
    if err != nil {
        return nil, nil, err
    }

    s.ConfigureLogger()
    go s.Start()

    if !s.ReadyForConnections(5 * time.Second) {
        return nil, nil, fmt.Errorf("NATS server did not start in time")
    }

    nc, err := nats.Connect(s.ClientURL())
    if err != nil {
        s.Shutdown()
        return nil, nil, err
    }

    return s, nc, nil
}
```

Key `server.Options` fields:
```go
type Options struct {
    // Network
    Host            string          // default: "0.0.0.0"
    Port            int             // default: 4222; -1 = random
    ClientAdvertise string          // advertised address (behind NAT/LB)
    HTTPHost        string          // monitoring host
    HTTPPort        int             // monitoring port (HTTP)
    HTTPSPort       int             // monitoring port (HTTPS)

    // Security
    Username        string
    Password        string
    Authorization   string          // token
    TLSConfig       *tls.Config
    AllowNewAccounts bool

    // JetStream
    JetStream       bool
    StoreDir        string          // REQUIRED for JetStream
    JetStreamMaxMemory int64
    JetStreamMaxStore  int64

    // Clustering
    ServerName      string          // unique per cluster
    Cluster         ClusterOpts
    Gateway         GatewayOpts
    LeafNode        LeafNodeOpts

    // Logging
    LogFile         string
    Syslog          bool
    Debug           bool
    Trace           bool
    TraceVerbose    bool
    Logtime         bool

    // Limits
    MaxConn         int
    MaxPayload      int32
    MaxPending      int64
    WriteDeadline   time.Duration

    // Accounts
    Accounts        []*server.Account
    SystemAccount   string
    NoAuthUser      string
}
```

### Monitoring from Embedded Server

```go
// Start HTTP monitoring
s.StartMonitoring()

// Get monitoring data programmatically
varz, err := s.Varz(&server.VarzOptions{})
connz, err := s.Connz(&server.ConnzOptions{Subs: server.SubsDetail})
jsz, err := s.Jsz(&server.JSzOptions{Streams: true, Consumers: true})

// Check JetStream status
if s.JetStreamEnabled() {
    cfg := s.JetStreamConfig()
    // cfg.StoreDir, cfg.MaxMemory, cfg.MaxStore
}
if s.JetStreamIsClustered() {
    leader := s.JetStreamIsLeader()
    peers := s.JetStreamClusterPeers()
}
```

## Docker Compose (3-Node Cluster)

```yaml
version: '3'
services:
  nats-1:
    image: nats:latest
    command: >
      --name nats-1
      --cluster nats://0.0.0.0:6222
      --routes nats://nats-2:6222,nats://nats-3:6222
      --js
      --sd /data
      --http_port 8222
    volumes:
      - nats1_data:/data
    ports:
      - "4222:4222"
      - "8222:8222"

  nats-2:
    image: nats:latest
    command: >
      --name nats-2
      --cluster nats://0.0.0.0:6222
      --routes nats://nats-1:6222,nats://nats-3:6222
      --js --sd /data
    volumes:
      - nats2_data:/data

  nats-3:
    image: nats:latest
    command: >
      --name nats-3
      --cluster nats://0.0.0.0:6222
      --routes nats://nats-1:6222,nats://nats-2:6222
      --js --sd /data
    volumes:
      - nats3_data:/data

volumes:
  nats1_data:
  nats2_data:
  nats3_data:
```

## Kubernetes (Helm Values)

```yaml
# values.yaml for nats/nats chart
cluster:
  enabled: true
  replicas: 3

nats:
  jetstream:
    enabled: true
    fileStore:
      enabled: true
      size: 10Gi
    memoryStore:
      enabled: true
      size: 1Gi

# NATS Box (debugging pod)
natsbox:
  enabled: true
```

## Cluster vs Leaf Node vs Gateway

| Feature | Cluster | Leaf Node | Gateway |
|---------|---------|-----------|---------|
| Topology | Full mesh | Hub-spoke | Hub-to-hub |
| Auth | Shared or per-cluster | Independent local auth | Per-gateway |
| JetStream | Shared domain | Local or bridge | Separate domains |
| Use case | HA, scale | Edge, IoT, multi-tenant | Geo-distributed superclusters |
| Inbound required | Yes | No | Yes |
| Max hops | 1 | 1 bridge hop | 1 bridge hop |
| Routing | Gossip mesh | Local-first | Interest-based filtering |

## JetStream Clustering — RAFT Details

- Meta group: all JetStream-enabled nodes; manages API and placement
- Stream group: one per stream; elected leader handles writes
- Consumer group: one per consumer; lives on stream member machines
- RAFT quorum = n/2 + 1 (3-node needs 2, 5-node needs 3)
- Without quorum: stream goes read-only (rejects new messages)
- Recommended: 3 nodes minimum, 5 for zone-failure tolerance
- Tag-based placement: `server_tags: ["zone:us-east-1a", "tier:storage"]`
- Stream placement: `Placement: &nats.Placement{Tags: []string{"zone:us-east-1a"}}`
