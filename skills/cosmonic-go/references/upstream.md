# Upstream references for Go components on Cosmonic Desktop

What each project is, why it is in the stack, and where to read when the skill is not
enough. Versions are the ones the skill's table verifies (2026-09-01).

## componentize-go — the compiler

- Repo + README: <https://github.com/bytecodealliance/componentize-go> — CLI reference
  (`build`, `bindings`, `-d`/`-w`, `componentize-go.toml` discovery), the async story.
- Releases: <https://github.com/bytecodealliance/componentize-go/releases> — v0.4.1 is
  the pinned release; `checksums.txt` beside each release. Assets:
  `componentize-go-{darwin-arm64,darwin-amd64,linux-amd64,linux-arm64}.tar.gz`,
  `componentize-go-windows-amd64.zip`. **No windows-arm64 asset.**
- main @ `20f3b0c` (2026-08-27): merge of PR #72 "update to wit-bindgen 0.61.1" — the
  verified head; PR #71 is what teaches it to emit `componentizego_async` itself.
- How it picks a Go (`pick_go` in `src/utils.rs`): `--go <path>`, else `go` on PATH,
  gated on Go ≥ 1.25 and, for async worlds, a `wasiOnIdle` hook in
  `<GOROOT>/src/runtime/lock_wasip1.go`; otherwise it downloads the dicej/go tag it
  hardcodes. The WASI p1→component adapter is embedded in the binary.

## wasmCloud Go SDK — `go.wasmcloud.dev/component`

- Repo: <https://github.com/wasmCloud/go> (the `component/` module; tags
  `component/vX.Y.Z`). v0.1.3 verified; v0.1.4 published 2026-08-31, unverified.
- Ships: the `wasmcloud:component-go` worlds (`wasip2@0.2.0` default = sync p2,
  `wasip3@0.2.0` = async p3), its own generated bindings (never regenerate), a `sleep`
  package for async-safe waiting, typed NATS helpers (`nats.Message`, `error` returns)
  for the `wasmcloud:nats` driver.
- Docs: <https://wasmcloud.com/docs/developer/languages/go/> — component development
  in Go on wasmCloud (the `wash`-centric flow; Desktop drives the same build).

## `go.bytecodealliance.org/pkg` — bindings runtime + `wasihttp`

- Repo: <https://github.com/bytecodealliance/go-modules> (module `go.bytecodealliance.org`).
  `pkg/wasihttp` is the `net/http` adapter; `pkg/wit/types` the `Result`/`Option` types
  the generated bindings use.
- Versions in play: `v0.2.4-0.20260806154504-91f6c4863e67` (pseudo-version, indirect
  via the SDK), `v0.2.3` (the `go-nats-*` starters), `v0.2.2` (what componentize-go
  v0.4.1's `bindings` writes). The same repo publishes the standalone **wit-bindgen-go**
  CLI (v0.7.0) — do not use it for async worlds: it emits zero functions.

## wit-bindgen — the generator inside componentize-go

- Repo: <https://github.com/bytecodealliance/wit-bindgen>. 0.61.1 is vendored at
  componentize-go main @ 20f3b0c and named in every generated file header. Rust
  components on Desktop use the wit-bindgen crate directly (0.60+ with `async-spawn`).

## The patched Go — `runtime.wasiOnIdle`

- The proposal: <https://github.com/golang/go/pull/76775> (unmerged). Stock Go cannot
  emit the component-model concurrency ABI; the patch adds the idle hook the async
  runtime needs.
- Prebuilt toolchains: <https://github.com/dicej/go/releases>, tag
  `go1.25.5-wasi-on-idle-v2` (the exact tag componentize-go v0.4.1 hardcodes):
  `go-{darwin-arm64,linux-amd64,linux-arm64,windows-amd64}-bootstrap.tbz`. No
  darwin-amd64, no windows-arm64 (cosmonic/desktop#433).

## WASI p3 / component model

- `wasi:http` 0.3.0 (the p3 `handler` world): <https://github.com/WebAssembly/wasi-http>.
- Component-model async (the `async-lift` / `task-return` ABI the verify step greps
  for): <https://github.com/WebAssembly/component-model/blob/main/design/mvp/Async.md>.
- `wasm-tools` (`component wit`, `print`): <https://github.com/bytecodealliance/wasm-tools> —
  Desktop's doctor installs it.

## Cosmonic Desktop

- `docs/GO-TOOLCHAIN-AIRGAP.md` in <https://github.com/cosmonic/desktop>: the air-gapped
  Go module (`go-mirror.tar`, `manifest.json`, the `bundle_go` / `go_wrapper` switches),
  the `~/.local/bin/componentize-go` wrapper and its guarantees, the offline
  environment, the RHEL 9 runbook.
- `docs/INSTALL.md` "Windows on ARM64": the `windows-amd64` build under emulation.
- The starters: `daemon/crates/cosmonicd/templates/go-http` (Shape A) and
  `templates/go-nats-*` (Shape B; each ships `docs/building.md`).
