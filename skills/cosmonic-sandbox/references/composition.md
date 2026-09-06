# Composing components with `wac` (compose-then-deploy)

Most workloads this skill builds need **no** composition: the host satisfies a
component's interfaces (`wasi:http`, keyvalue, messaging, …) through
`hostInterfaces`, so you build **one** component and deploy it. Composition is the
exception — reach for it only when one **guest** component must be wired to
**another guest** component, a link the host model can't express. This reference is
the compose-then-deploy recipe for those cases.

## When to compose (and when NOT to)

**Compose when the wiring is guest→guest:** one guest component imports an interface
that **another guest component exports**, and you want them linked into a single
deployable artifact.

- **Cross-component streaming.** Passing a live `wasi:io/streams` `input-stream`
  **between** components — one exports, say, `calc(input-stream) -> u64`, another
  hands it the request body — is the strongest reason to compose. Across a **runtime
  link** (two components in one Workload, wired by the host) that live handle does
  **not** transfer today: it fails with HTTP 500 (`wac`-composing them in-process is
  the fix, or pass `list<u8>` bytes instead of a live stream). **`wac`-composed into
  one component it works** — the stream crosses in-process via the canonical ABI.
  (Verified on Desktop: 512 KB across a composed boundary, an **all-Rust** stack and
  an **all-`componentize-go`** stack. Build Go streaming components with componentize-go,
  per the **cosmonic-go** skill; no other Go compiler was verified here.)
- **Bring-your-own capability as a component.** You wrote a small adapter/provider
  **component** (not a host capability) and want a guest to call it in-process.
- **Split one app across components** with a private internal interface between them.

**Do NOT compose when the host already provides the capability.** If your component
imports `wasi:http`, `wasi:keyvalue`, `wasi:blobstore`, `wasi:messaging`, NATS
(`wasmcloud:nats`), etc., that is a **host** interface — list it in `hostInterfaces`
(see `crds.md`), don't compose a provider in. Composing a capability the host would
have supplied only makes the artifact bigger and the graph harder to reason about.

## The tool: `wac`

`wac` is the WebAssembly Composition CLI (`bytecodealliance/wac`). Two modes:

- **`wac plug`** — the common case: satisfy one **socket** (importer) component's
  imports with one or more **plug** (exporter) components. Exact, no WAC file needed.
- **`wac compose`** — a `.wac` source file describing a multi-component graph with
  explicit wiring (renaming interfaces, fan-in/out, re-exporting a subset). Reach for
  it when `wac plug` can't express the graph.

**Availability — check `wac --version` first.** Whether the Preflight doctor provisions
`wac` depends on the Desktop build:

- **Today / current releases: the doctor does NOT provision `wac`.** It isn't in the
  Desktop toolchain lock yet, so the doctor doesn't manage or install it — online or
  offline. **Install it yourself:** `cargo install wac-cli` (the crate; the binary is
  `wac`) or a release binary from the wac repo.
- **Once the signed-`wac` toolchain work ships** (cosmonic/desktop#434, the release that carries it),
  the doctor provisions `wac` as an **advisory** signed OCI tool, exactly like
  `wash`/`wkg`/`wasm-tools` — pulled from the signed registry when online, or imported
  from the in-app toolchain bundle when offline (present in the air-gapped installers,
  independent of the Go mirror). "Advisory" means a **missing `wac` never blocks** a
  Rust/JS build, so it may not be installed until you build something that needs it or
  run the doctor (Settings → Doctor); if `wac` isn't on PATH, run the doctor or install
  it yourself as above.

Confirm the exact flags on your version with `wac plug --help` / `wac compose --help`;
the surface below matches `wac` 0.8+ (identical through 0.10).

## Compose-then-deploy recipe

1. **Build each component** the normal way (`cosmonic_dev_start` / `wash build`, per
   `SKILL.md` step 4). Each is a standalone `.wasm`. The importer's imports must match
   the exporter's exports **exactly** — same package, interface, and **version**
   (`wasi:io/streams@0.2.3` ≠ `@0.2.0`). A version skew is the #1 cause of a plug that
   "does nothing."
2. **Confirm the wiring** before composing. Read each component's world with
   `cosmonic_image_inspect <ref>` (or `wasm-tools component wit <file>.wasm`): the socket
   must import precisely what the plug exports.
3. **Compose into one artifact.**
   - Common case (`wac plug`) — the socket is the importer, `--plug` the exporter:
     ```bash
     wac plug ./consumer.wasm --plug ./producer.wasm -o ./composed.wasm
     ```
     `--plug` may be repeated to satisfy imports from several exporters.
   - Graph case (`wac compose`) — author a `.wac` file, then:
     ```bash
     wac compose ./composition.wac -o ./composed.wasm
     ```
     A `.wac` file is schematically: declare the package, instantiate each component
     (`new pkg:name { … }`) wiring one instance's imports to another's exports, and
     `export` the instance whose interface the composed component should expose. Keep
     the exact syntax to `wac`'s WAC-language docs — start from an example in the wac
     repo rather than hand-writing it, and let `wac compose` validate.
4. **Deploy the composed artifact like any single component** (`SKILL.md` steps 5–7):
   push `composed.wasm` to `oci.localhost:8200/apps/<name>:<tag>` (Windows before 10 1709:
   `oci.localhost.cosmonic.sh:8200/...`), author a `Workload` referencing that image,
   and `cosmonic_workload_apply`.

   **Key subtlety — what still goes in `hostInterfaces`.** Composition satisfies only
   the **guest→guest** imports. Everything the composed component exchanges with the
   **host** still goes in `hostInterfaces`: the host-driven `wasi:http` trigger it
   **exports** as its entrypoint (declared in `hostInterfaces` even though it's an
   export — see `crds.md`), plus any `wasi:keyvalue` / outbound `wasi:http` / NATS the
   components **import**. Compose the private wiring; let the host satisfy the rest.
5. **Verify** as usual — smoke-test the URL (`patterns-and-smoke-tests.md`). If a
   composed streaming path returns empty or truncates, re-check the interface **version**
   match (step 1) and that the streaming side was built with componentize-go (what `go-http`
   scaffolds); a Go component from another compiler is not a verified path here.

## Trust & safety

- **Composition doesn't widen the sandbox.** The composed artifact is still one
  sandboxed component: the guest→guest wiring lives entirely inside the Wasm boundary,
  and egress stays **deny-by-default** at the host — grant hosts only via
  `localResources.allowedHosts` on the composed component's Workload, exactly as for a
  single component.
- **Grant capabilities on the composed component, not the parts.** Because only the
  composed artifact is deployed, its `hostInterfaces` + `allowedHosts` are the whole
  capability surface. Don't over-grant to "make the plug work" — a plug needing no
  network shouldn't drag an allowlist entry into the deployment.
- **Local unsigned image:** for local dev the same
  `desktop.cosmonic.com/unsafe-allow-unsigned: "true"` annotation applies; never carry
  it into a published/Control deployment.

## Quick reference

| You have… | Do |
|---|---|
| One importer, one (or a few) exporter(s) | `wac plug ./importer.wasm --plug ./exporter.wasm -o composed.wasm` |
| A multi-component graph / renamed wiring | author `composition.wac` → `wac compose composition.wac -o composed.wasm` |
| A capability the **host** provides (http, keyvalue, nats, …) | **don't compose** — declare it in `hostInterfaces` (`crds.md`) |
| To read a component's imports/exports | `cosmonic_image_inspect <ref>` or `wasm-tools component wit <file>.wasm` |
