# Upgrading a p3 Rust component's async runtime

Read this only when a Rust component pins `wasip3` 0.7.x, or `wit-bindgen` 0.58 or older with the
`inter-task-wakeup` feature. Those versions carry an older async runtime (the code `wit-bindgen`
generates to schedule a component's async tasks) that current Cosmonic hosts reject. The fix is a
version bump and a rebuild.

**Installed from Launchpad?** If the failing app came from Launchpad (Desktop's catalog of
ready-made apps), there is nothing to edit. Reinstall it from Launchpad to get the current build.

## Symptom

The component builds and deploys, then fails when it handles a request:

- `cosmonic_logs_query` (or `cosmonic_dev_logs`) shows a trap, meaning the host stopped the
  component in the middle of a call, with the message
  `waitable cannot be used synchronously while added to a waitable set`.
- Callers get HTTP 500. If the component sets `poolSize` (the number of warm instances the host
  keeps ready between calls), the host also logs `pooled instance dropped the request`. Without
  `poolSize` that line does not appear, so look for the trap message instead.

## Confirm

From the project directory:

```bash
cargo tree --target all -e features -i wit-bindgen
```

This prints the `wit-bindgen` version in the build and the crates that turn on each of its
features. The project needs the upgrade if it shows `wit-bindgen` 0.58 or older with
`inter-task-wakeup`, or `wasip3` 0.7.x above it. If Cargo answers that the name is ambiguous, the
build has two `wit-bindgen` versions, which means the direct dependency is out of step with
`wasip3`; run it once per version with `-i wit-bindgen@<version>`.

Do not judge from the `producers` metadata in the built `.wasm` (the list of tools a component
records about itself). It does not reliably name the `wit-bindgen` version that built it.

## Fix

1. In `Cargo.toml`, set `wasip3` to `0.9.0` and the direct `wit-bindgen` to `0.62.0`, keeping its
   features. The direct `wit-bindgen` must be the exact version `wasip3` uses, so Cargo builds one
   runtime. A project that depends on `wit-bindgen` with no `wasip3` (the Kafka templates, for
   example) bumps only `wit-bindgen`.
2. Rename `wit_bindgen::spawn` to `wit_bindgen::spawn_local`, including calls written as
   `wasip3::wit_bindgen::spawn`.
3. Build. Cargo rewrites `Cargo.lock` to match the new versions. Run the `cargo tree` command above
   again and check that it lists only `wit-bindgen` 0.62.0.
4. Redeploy. In a dev session `cosmonic_dev_start` picks up the rebuild. For a deployed Workload,
   push under a new image tag and re-apply; a reused tag keeps serving the old build.

Components rebuilt this way also run on older Cosmonic hosts that embed Wasmtime 47 (the
WebAssembly runtime inside the host), so the rebuild does not have to match a particular Desktop
version.
