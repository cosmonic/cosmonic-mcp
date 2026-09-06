# Cosmonic HTTP Trigger

The **HTTP Trigger** is the preferred way to expose a component over HTTP. It bundles the
component(s) + ingress host/paths into one resource and **derives the `wasi:http/incoming-handler`
host interface for you** from `spec.ingress.host`, so you don't hand-write the http `hostInterfaces`
block.

- **apiVersion:** `control.cosmonic.io/v1alpha1`
- **kind:** `HTTPTrigger`

It works in two places:

1. **Cosmonic Desktop (local)**: the MCP tools have **no path that applies an `HTTPTrigger`**.
   `cosmonic_workload_apply` accepts only a **flat `Workload`** (it does *not* unwrap the
   `spec.template.spec` of an `HTTPTrigger`/`WorkloadDeployment`; that fails with
   `missing field 'components'`), and `cosmonic_workload_draft` drafts a `Workload` **only from an OCI
   image ref or a git repo URL** (its single `source` argument), not from a manifest. So on Desktop,
   express HTTP ingress by authoring the flat `Workload` with an http `hostInterfaces` block directly
   (see `crds.md` / the skill's step 6). The `HTTPTrigger` manifest below is for **Cosmonic Control**.
2. **Cosmonic Control (Kubernetes)**: deploy via the `http-trigger` Helm chart (the operator
   reconciles the same CRD; Traefik → Envoy route the host).

**Jump to:** [HTTPTrigger manifest (Control)](#httptrigger-manifest-cosmonic-control) ·
[Helm chart (Control)](#helm-chart-cosmonic-control-only) ·
[HTTPTrigger vs raw Workload](#choosing-httptrigger-vs-raw-workloadworkloaddeployment)

## HTTPTrigger manifest (Cosmonic Control)

```yaml
apiVersion: control.cosmonic.io/v1alpha1
kind: HTTPTrigger
metadata:
  name: my-app                 # = the app/DNS name
  namespace: default
spec:
  ingress:
    host: my-app.localhost   # reachable at http://my-app.localhost:8200/
    paths:                               # optional; defaults to "/" Prefix
      - path: /
        pathType: Prefix
  replicas: 1                  # optional
  # timeout: 300s              # optional
  template:
    spec:
      components:
        - name: my-app
          image: oci.localhost:8200/apps/my-app:0.1.0   # local registry (Windows before 10 1709: oci.localhost.cosmonic.sh:8200), or ghcr.io/... when published
      # Add extra capabilities here (NOT the inbound http one; that's derived from ingress.host):
      # hostInterfaces:
      #   - namespace: wasi
      #     package: http
      #     interfaces: [outgoing-handler]   # for calling upstream APIs
```

> **Control vs Desktop for outbound:** on **Cosmonic Control** an `HTTPTrigger` declares the outbound
> `wasi:http` interface in `hostInterfaces` as shown below. On **Cosmonic Desktop** (the flat
> `Workload` path) outbound is **implicit** — you set only `localResources.allowedHosts`, no
> `outgoing-handler`/`client` entry (see `crds.md` / `recipes.md` §6b).

Outbound HTTP example (Control; pair with `allowedHosts`):
```yaml
spec:
  ingress: { host: wasmstreet.localhost }
  template:
    spec:
      hostInterfaces:
        - namespace: wasi
          package: http
          interfaces: [outgoing-handler]
        - namespace: wasi
          package: logging
          interfaces: [logging]
      components:
        - name: wasmstreet
          image: ghcr.io/cosmonic-labs/wasmstreet:0.1.0
          localResources:
            allowedHosts: ["query1.finance.yahoo.com"]   # hostname only, no scheme
```

Service-sidecar example:
```yaml
spec:
  ingress: { host: ocelaudit.localhost }
  template:
    spec:
      service:
        name: csl-service
        image: ghcr.io/cosmonic-labs/ocelaudit-csl-service:0.1.0
      components:
        - name: api-gateway
          image: ghcr.io/cosmonic-labs/ocelaudit/api-gateway:0.1.0
```

Only **one** component may export `wasi:http/incoming-handler`.

## Helm chart (Cosmonic Control only)

Chart: `oci://ghcr.io/cosmonic-labs/charts/http-trigger` (also vendored at
`cosmonic-labs/control-demos/charts/http-trigger`). It renders exactly one `HTTPTrigger` CR.

`values.yaml` keys:
```yaml
hostSelector: {}             # host-label placement, e.g. {hostgroup: "default"}
hostInterfaces: []           # extra WASI interfaces (namespace/package/version/interfaces)
volumes: []
components: []               # name + image; one must export wasi:http/incoming-handler
replicas: 1
ingress:
  host: ""                   # e.g. my-app.localhost
  paths:
    - path: /
      pathType: Prefix
pathNote: ""                 # cosmetic suffix appended to the printed URL (e.g. "/ui")
```

Install (Control):
```bash
helm install my-app oci://ghcr.io/cosmonic-labs/charts/http-trigger \
  --set ingress.host=my-app.localhost \
  -f values.http-trigger.yaml
```

On Control the app is served on standard ports 80/443 via Traefik (not `:8200`); the control-plane
install must also know the host (`--set 'ingress.hosts[0].host=my-app.localhost'`).

## Choosing: HTTPTrigger vs raw Workload/WorkloadDeployment

- **HTTP app, want it at a hostname?** → `HTTPTrigger` (cleanest; derives ingress).
- **Need replicas/scaling but still HTTP?** → `HTTPTrigger` with `replicas`, or a
  `WorkloadDeployment` with an explicit http `hostInterfaces` block.
- **Non-HTTP (messaging-only, TCP service, batch)?** → `Workload` / `WorkloadDeployment`
  (see `crds.md`).
