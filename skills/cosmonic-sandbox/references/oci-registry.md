# Local dev OCI registry

**Cosmonic Desktop ships this registry built in.** It is the `oci-registry` **system workload**
(the user can see it under Settings → Admin Workloads) and it is already running whenever the
Desktop daemon is — **never deploy a registry workload of your own to get a local push target**.
It is reached through the Desktop ingress on port `8200`; the router strips the port and routes by
Host header, answering the registry on three names: `oci`, `oci.localhost`, and
`oci.localhost.cosmonic.sh`.

**Canonical push ref (macOS/Linux): `oci.localhost:8200/apps/<name>:<tag>`.** This is the default
for every step in this skill.

> **Windows before 10 1709** (or a resolver that does not special-case `.localhost`): `oci.localhost`
> fails with "No such host is known" in most tools (`wash`, `oras`, `docker` — and in the daemon
> itself, which is what resolves the ref you hand to `cosmonic_project_publish`). On Windows push and
> reference images as **`oci.localhost.cosmonic.sh:8200/apps/<name>:<tag>`** — the public wildcard
> resolves to `127.0.0.1`. (It needs working DNS; the Host-header probe below does not.)

> Alternative: a standalone **`wash dev` registry on `localhost:8080`** (the upstream default) may
> exist on some machines. Only use `:8080` if you've confirmed that's where the registry is serving.

**Confirm where it's serving before pushing**: the registry is the endpoint that answers `GET /v2/`
with header `docker-distribution-api-version: registry/2.0`:
```bash
curl -s -H 'Host: oci' http://127.0.0.1:8200/v2/_catalog                        # Desktop (any OS)
curl -s -o /dev/null -w '%{http_code}\n' http://localhost:8080/v2/                       # standalone wash dev
```
If the Desktop probe 404s, the daemon is either not running or too old to ship the built-in
registry — tell the user, and don't work around it by deploying your own registry.

**Repository paths need at least two segments** (e.g. `apps/my-app`, not just `my-app`); a
single-segment push fails with a 404 on `…/blobs/uploads/`.

Reference the pushed image from your Workload exactly as you pushed it
(`image: oci.localhost:8200/apps/<name>:<tag>`, on Windows before 10 1709 the `oci.localhost.cosmonic.sh:8200`
form); the daemon digest-pins it on apply.

The registry is itself a wasmCloud component, but it ships **inside Cosmonic Desktop** as a
read-only system workload (namespace `cosmonic-system`) that the Desktop seeds and upgrades —
it never appears in your workload list, and you cannot apply, modify, or delete it through the
API. It's a reactor that scales to zero and persists blobs/manifests to a host disk volume, so
pushed images survive restarts.

## Pushing

`cosmonic_project_publish` pushes for you (preferred: it returns a durable Workload draft):
```
cosmonic_project_publish(project_id="<id>", reference="<name>:0.1.0", confirm=true)
```
A reference with no registry goes to the built-in registry (never Docker Hub), and a loopback target
(the built-in registry, `localhost`, `127.0.0.1`) is pushed over plain HTTP automatically —
`insecure=true` is only needed for a non-loopback plain-HTTP registry.

Manual push (any OCI client works; note the **two-segment** repo path `apps/<name>`):
```bash
# Desktop registry (default; macOS/Linux)
wash oci push --insecure oci.localhost:8200/apps/<name>:0.1.0 ./component.wasm
# Windows before 10 1709 — bare .localhost subdomains may not resolve; same registry, public alias:
wash oci push --insecure oci.localhost.cosmonic.sh:8200/apps/<name>:0.1.0 ./component.wasm
# oras also works: oras push --plain-http oci.localhost:8200/apps/<name>:0.1.0 ./component.wasm:application/wasm
```

## Re-deploying an edited build: bump the tag

**Re-pushing an edited build under a tag you already deployed keeps the old build live, and
restarting the workload or the daemon does not change that.** On apply the daemon resolves the tag to
a digest and pins the Workload to it (see above), so a Workload applied at `:0.1.0` stays on the
content that tag first resolved to; re-pushing new bytes under `:0.1.0` and re-applying the unchanged
spec does not pick them up. (Whether the stale layer is held by the daemon's pin or cached by the
registry, the fix is identical.) So on every rebuild you want live, push under a **new tag** and point
the Workload at it:

```bash
wash oci push --insecure oci.localhost:8200/apps/<name>:0.1.1 ./component.wasm   # was :0.1.0
# then set the Workload image to ...:0.1.1, re-apply (cosmonic_workload_apply), and re-verify
```

Increment the tag each iteration (`:0.1.0` → `:0.1.1` → …); a new image ref forces the daemon to
resolve fresh content. Reusing a tag is the usual cause of "I pushed a fix but Desktop still runs the
old code," and a status-only smoke test won't catch it, so when you re-verify, assert the thing that
changed (a new field or body), not just the status code. Path A sidesteps the whole issue:
`cosmonic_dev_start` rebuilds and runs locally without a registry push, and `cosmonic_project_publish` returns a
**digest-pinned** draft each time, so applying that fresh draft targets the exact content it just
pushed rather than a reused tag.

## Browsing

Against `:8080` directly, or via the ingress with `-H 'Host: oci'` (any OS) on `:8200`:
```bash
curl <registry>/v2/_catalog              # repositories
curl <registry>/v2/<ns>/<name>/tags/list # tags
curl <registry>/healthz                  # health
# Web UI at /  and JSON browse at /api/repositories
```

## Note on local/unsigned images

Images pushed to the local registry are plain-HTTP and unsigned. If the daemon refuses an unsigned
local image, add the Desktop annotation to the workload metadata:
```yaml
metadata:
  annotations:
    desktop.cosmonic.com/unsafe-allow-unsigned: "true"
```
