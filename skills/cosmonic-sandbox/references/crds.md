# wasmCloud v2 CRD reference (`runtime.wasmcloud.dev/v1alpha1`)

Authoritative for Cosmonic Desktop and portable to Cosmonic Control. The docs site labels this
**"v2"**, but the **apiVersion string is always `runtime.wasmcloud.dev/v1alpha1`**; there is no
`v2` apiVersion. The HTTP Trigger CRD lives in a different group (`control.cosmonic.io/v1alpha1`,
see `http-trigger.md`).

**Five CRDs:** `Artifact`, `Host`, `Workload`, `WorkloadDeployment`, `WorkloadReplicaSet`.
There are **no** `Link`, `Provider`, `Config`, or `HostGroup` CRDs; those are v1/wadm-era concepts.
`hostgroup` is just a label (`metadata.labels.hostgroup`) that workloads target via
`spec.hostSelector.hostgroup`.

**Applying manifests (verified against the daemon):**
- `cosmonic_workload_apply` accepts a **flat `Workload` JSON object only** (top-level
  `spec.components` + `spec.hostInterfaces`). Passing a `WorkloadDeployment` or `HTTPTrigger` (which
  nest under `spec.template.spec`) fails with `missing field 'components'`.
- The `cosmonic_workload_draft` **MCP tool** drafts a `Workload` from an **OCI image ref or a git repo
  URL only** (its single `source` argument); it does **not** accept an `HTTPTrigger`/`WorkloadDeployment`
  manifest as input. So through the MCP tools there is no path that applies those nested CRDs on
  Desktop — author the flat `Workload` directly. Use `synthesize` to draft a `Workload` for an
  existing published component you have no local source for.
- The `projects` MCP tools (`cosmonic_template_list`, `cosmonic_project_create`, `cosmonic_dev_start`,
  `cosmonic_project_publish`) are gated behind a **Labs feature flag**; if disabled they return
  `feature_disabled`: enable in Settings → Labs or relaunch with `COSMONIC_FLAG_PROJECTS=1`. The
  `wash` build/push path needs no flag.

---

**Jump to:** [which kind to author](#which-kind-to-author) · [Workload](#workload) ·
[WorkloadDeployment](#workloaddeployment) · [Artifact](#artifact) ·
[WorkloadReplicaSet](#workloadreplicaset) · [Host](#host) ·
[HTTP ingress & routing](#http-ingress--hostname-routing-desktop) ·
[status values](#workload-status-values)

## Which kind to author

| Goal | Author |
|---|---|
| Single app, no scaling controls | `Workload` |
| App with replicas / rolling updates / `kubectl scale` / HPA / KEDA | `WorkloadDeployment` |
| HTTP app you want exposed at a hostname (the common case) | `HTTPTrigger` (see `http-trigger.md`): derives the HTTP ingress for you |
| Operator-watched OCI image with auto rolling updates on new tags | `Artifact` + reference it from a `WorkloadDeployment` |
| Manual replica-set control | `WorkloadReplicaSet` |

---

## Workload

```yaml
apiVersion: runtime.wasmcloud.dev/v1alpha1
kind: Workload
metadata:
  name: time-api            # DNS label, <=63 chars, [a-z0-9][a-z0-9-]*
  namespace: default
  # Cosmonic Desktop annotations (optional):
  #   desktop.cosmonic.com/enabled: "true"            # keep workload running
  #   desktop.cosmonic.com/unsafe-allow-unsigned: "true"  # allow unsigned/local images
  #   desktop.cosmonic.com/source-hash: "<hash>"      # provenance
spec:
  components:                # required, >=1
    - name: api              # DNS label
      image: ghcr.io/acme/time-api:0.1.0   # OCI ref; pinned to sha256:... at apply
      imagePullPolicy: Always               # optional
      poolSize: 8            # warm instances kept between calls. P3-ONLY (see "Warm pool" below):
                            #   a P2 `incoming-handler` accepts this and silently ignores it.
      maxConcurrency: 1      # concurrent calls one warm instance serves (2.7.0+; default 1).
                            #   Raise only for a guest that yields on I/O, not one that blocks.
      maxInvocations: 0      # retire a warm instance after N calls (0 = never). NOT a concurrency
                            #   cap (that's maxConcurrency); it bounds instance reuse/staleness.
      localResources:        # optional, see below
        environment:
          config:
            LOG_LEVEL: info
        allowedHosts: []     # egress allow-list: EMPTY = DENY ALL (fail-closed)
  service:                   # optional long-running sidecar (e.g. TCP service)
    name: csl-service
    image: ghcr.io/acme/csl-service:0.1.0
    maxRestarts: 5
  hostInterfaces:            # capabilities the host grants (see below)
    - namespace: wasi
      package: http
      interfaces: [handler]  # p3 wasi:http/handler@0.3.0, matching poolSize above.
                            #   A p2 component uses [incoming-handler] and should omit poolSize.
      config:
        host: time-api.localhost   # hostname for HTTP ingress routing
  volumes:
    - name: cache
      ephemeral: {}
```

### Warm instance pool (performance)

By default every call gets a fresh `wasmtime::Store` (hermetic, then dropped). `poolSize` keeps warm
instances parked between calls — **but only on the HTTP ingress path for a P3 component that exports
`wasi:http/handler@0.3.0`**. A P2 `wasi:http/incoming-handler@0.2.x` component accepts `poolSize` in
the manifest and silently throws its instance away after every request.

Measured on a static hello-world ([`cosmonic-labs/ex-hello-pool-size-p3`](https://github.com/cosmonic-labs/ex-hello-pool-size-p3), runtime 2.6.1):

| Workload                          | req/s | p50   | p99   |
|-----------------------------------|-------|-------|-------|
| P2 `incoming-handler`, poolSize 1 | 18.6k | 2.4ms | 8.7ms |
| P3 `handler`, poolSize 0          | 19.1k | 2.4ms | 6.9ms |
| P3 `handler`, poolSize 128        | 57.0k | 0.8ms | 2.7ms |

P3 alone buys nothing (19.1k vs 18.6k is noise); the 3× comes from a real `poolSize` on a P3
component. So for throughput: **export the P3 `handler`** (the `rust-http` and `rust-mcp` scaffolder
templates already do) and **set a meaningful `poolSize`** (a few dozen up to ~128 for concurrent
HTTP). Set it explicitly on any component you want kept warm — treat an unset `poolSize` as no pool
(the `poolSize 0` baseline above). `maxConcurrency` (2.7.0+) multiplies each warm instance's capacity for a guest that yields on
I/O — total warm capacity ≈ `poolSize × maxConcurrency`. Pooled instances share process-global
memory, so keep them panic-free: a trap faults every in-flight call on that instance.

### `localResources` (per component or service)

```yaml
localResources:
  memoryLimitMb: 128         # optional
  cpuLimit: 1                # optional
  environment:
    config:                  # literal env vars
      LOG_LEVEL: info
    configFrom:              # named config sources
      - name: my-config
    secretFrom:              # named secret references (register with cosmonic_secret_set)
      - name: db-password
  volumeMounts:
    - name: cache
      mountPath: /data
      readOnly: false
  allowedHosts:              # outbound HTTP allow-list (p2 wasi:http/outgoing-handler or p3 client)
    - api.example.com
    - "*.s3.amazonaws.com"
  allowedIpNameLookups:      # names resolvable via wasi:sockets/ip-name-lookup (deny-all default)
    - "*.internal.example"   #   "*" = any; "host"; "*.suffix"; or a literal IP. No scheme/port/path.
  allowedHostLoopbackPorts:  # ports on the machine's own loopback reachable via host.wasmcloud.internal
    - "5432"                 #   "5432" (TCP) | "5432/tcp" | "53/udp"
```

- **`allowedIpNameLookups`** gates `wasi:sockets/ip-name-lookup` (raw-socket name resolution),
  separately from `allowedHosts` (which governs outbound `wasi:http`). It is also deny-by-default:
  an empty/absent list fails every lookup. A p2 HTTP-only component that only calls
  `wasi:http/outgoing-handler` needs `allowedHosts`, not this.
- **`allowedHostLoopbackPorts`** lets a sandboxed component reach a service on the machine's own
  loopback (e.g. a local Postgres on `5432`) through the reserved name `host.wasmcloud.internal`.
  Deny-by-default; list only the ports it needs.

- **`allowedHosts` is deny-by-default (fail-closed) on Cosmonic Desktop.** An empty or absent list
  blocks **all** outbound HTTP. List every host the component dials out to (hostname only, no scheme,
  no path; matched case-insensitively). This is the opposite of some upstream docs; trust the
  Desktop behavior.
- **Never inline secret values.** Register a reference with `cosmonic_secret_set`, then point
  `secretFrom: [{ name }]` at it.

### `hostInterfaces`

Each entry needs `namespace`, `package`, `interfaces`; optional `version`, `name`, `config`,
`configFrom`, `secretFrom`. Use `cosmonic_image_inspect <image>` to read a component's WIT world and wire
the exact interfaces it imports/exports.

Common interfaces:

```yaml
hostInterfaces:
  # Inbound HTTP (the HTTP trigger). config.host sets the ingress hostname.
  # Use `[handler]` for a P3 (wasi:http@0.3.0) component (warm-pool capable, see above)
  # or `[incoming-handler]` for P2. The daemon binds whichever the component exports;
  # the rust-http and rust-mcp scaffolder templates are P3 `handler`.
  - namespace: wasi
    package: http
    interfaces: [incoming-handler]
    config:
      host: my-app.localhost
      # host-aliases: "alt-one.localhost,alt-two.localhost"   # optional extra hostnames

  # Outbound HTTP is NOT declared as a hostInterface on Cosmonic Desktop — it is provided
  # implicitly and gated only by `localResources.allowedHosts` on the component (deny-by-default).
  # A p2 component imports `wasi:http/outgoing-handler`; a p3 component imports `wasi:http/client`;
  # either way you set `allowedHosts`, not an entry here.

  # Key-value store
  - namespace: wasi
    package: keyvalue
    interfaces: [store, atomics, batch]

  # Messaging
  - namespace: wasmcloud
    package: messaging
    version: "0.2.0"
    interfaces: [consumer, types]

  # Logging
  - namespace: wasi
    package: logging
    interfaces: [logging]
```

**Multi-backend binding:** when a component needs two implementations of the same
`namespace`+`package` (e.g. two `wasi:keyvalue` backends), give each entry a unique `name`
(DNS-label style, `[a-z0-9][a-z0-9-]*`); that name maps to the identifier passed to resource-opening
calls like `store::open("cache")`. `name` is optional otherwise.

---

## WorkloadDeployment

A scalable wrapper around a Workload. Implements the Kubernetes `/scale` subresource, so
`kubectl scale`, HPA, and KEDA work against it. `spec.replicas` defaults to 1. The daemon unwraps
`.spec.template.spec` (a full WorkloadSpec).

```yaml
apiVersion: runtime.wasmcloud.dev/v1alpha1
kind: WorkloadDeployment
metadata:
  name: hello-world
  namespace: default
spec:
  replicas: 3
  deployPolicy: RollingUpdate          # optional
  artifacts:                            # optional: bind Artifact(s) into the template
    - name: http-component
      artifactFrom:
        name: http-hello-world          # references an Artifact resource
  template:
    metadata:
      labels:
        app: hello-world
    spec:
      hostSelector:
        hostgroup: default
      components:
        - name: http-component
          image: ghcr.io/wasmcloud/components/http-hello-world-rust:0.1.0
          # (no poolSize: this stock image is a p2 incoming-handler, which ignores it)
      hostInterfaces:
        - namespace: wasi
          package: http
          interfaces: [incoming-handler]
          config:
            host: hello-world.localhost
```

---

## Artifact

Wraps an OCI image (with optional pull secret), fetches it into a NATS JetStream Object Store,
tracks revisions under `Status.ArtifactURL`, and auto-triggers rolling updates when a new image is
detected. Reference it from a `WorkloadDeployment` via `spec.artifacts[*].artifactFrom.name`.

```yaml
apiVersion: runtime.wasmcloud.dev/v1alpha1
kind: Artifact
metadata:
  name: http-hello-world
  namespace: default
spec:
  image: ghcr.io/wasmcloud/components/http-hello-world-rust:0.1.0
  imagePullSecret:
    name: ghcr-secret        # optional, for private registries
```

---

## WorkloadReplicaSet

Ensures N replicas run at once. Normally owned by a WorkloadDeployment; usable directly for granular
control. Same `spec.template.spec` shape as a Workload.

```yaml
apiVersion: runtime.wasmcloud.dev/v1alpha1
kind: WorkloadReplicaSet
metadata:
  name: hello-world-v1
  namespace: default
spec:
  replicas: 5
  template:
    metadata:
      labels: { app: hello-world, version: v1 }
    spec:
      hostSelector: { hostgroup: default }
      components:
        - name: http-component
          image: ghcr.io/wasmcloud/components/http-hello-world-rust:0.1.0
          # (no poolSize: this stock image is a p2 incoming-handler, which ignores it)
      hostInterfaces:
        - namespace: wasi
          package: http
          interfaces: [incoming-handler]
          config: { host: hello-world.localhost }
```

---

## Host

Defines a runtime host. Note: **no `spec` wrapper**: fields are at the resource root. You rarely
author this on Desktop (the daemon provides a `default` hostgroup).

```yaml
apiVersion: runtime.wasmcloud.dev/v1alpha1
kind: Host
metadata:
  name: host-sample
  namespace: default
  labels:
    hostgroup: default
hostId: NABCDEFGHIJKLMNOPQRSTUVWXYZ234567
hostname: host-sample.default
httpPort: 4000
```

---

## HTTP ingress & hostname routing (Desktop)

- The Desktop HTTP ingress listens on **`127.0.0.1:8200`**.
- The daemon's router reads the request **`Host` header, strips any `:port`**, and matches the bare
  hostname against each workload's `hostInterfaces[].config.host` (or an `HTTPTrigger`'s
  `ingress.host`).
- So a workload with `config.host: my-app.localhost` is reachable at:
  ```bash
  curl http://my-app.localhost:8200/
  # equivalently, and on every OS regardless of DNS:
  curl -H 'Host: my-app.localhost' http://127.0.0.1:8200/
  ```
- **The ingress name is `<name>.localhost`.** Set `config.host: <name>.localhost`; the workload is
  reachable at `http://<name>.localhost:8200/` (`.localhost` names resolve to loopback on macOS, Linux,
  and Windows 10 1709+ / 11 with no `/etc/hosts` edits). Report the URL from the `config.host` the
  applied Workload carries, and use the Host-header form above to verify routing, or as the URL on
  Windows before 10 1709 / behind a resolver that does not special-case `.localhost`. An MCP server
  must list its host in `MCP_ALLOWED_HOSTS`.
- One `config.host` is global across namespaces — two workloads cannot share it.
- Deploys do a blue-green cutover; HTTP invocations are counted per workload for scaling.

## Workload status values

`pending` → `starting` → `running` (also `completed`, `stopping`, `stopped`, `failed`).
Check with `cosmonic_workload_list`; a `failed` workload reports `restarts` and a `message`.
