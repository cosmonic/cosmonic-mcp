---
name: cosmonic-go
description: Build a Go WebAssembly component for Cosmonic Desktop with componentize-go on async WASI p3 (wasi:http/handler@0.3.0, wasmcloud:nats) — the known-working toolchain versions (componentize-go v0.4.1 / main @ 20f3b0c, wasmCloud Go SDK component/v0.1.3, the patched Go that carries runtime.wasiOnIdle), how to install componentize-go on macOS, Linux, Windows and Windows-on-ARM64, the two project shapes (SDK — build only; WIT-first — bindings then build), the async build rules (-w world pin, GOFLAGS=-tags=componentizego_async), the gotcha catalogue (wit-bindgen-go emits zero functions, go.mod rewritten, world merge, offline GOPROXY=off), and what Desktop's air-gapped Go module and its componentize-go wrapper do. Triggers on Go or TinyGo — any Go component, the go-http and go-nats-* starters, a Go build failure, "componentize-go", "wit-bindgen-go", "Go wasm component", or "which Go/componentize-go version". Pair with cosmonic-sandbox (the build and deploy loop) and cosmonic-nats (NATS patterns).
license: Apache-2.0
compatibility: Cosmonic Desktop 0.5.27+ (go-http on componentize-go + the wasmCloud Go SDK; the fourteen go-nats-* starters). A lean (online) install provisions no Go toolchain — you install Go 1.25+ and componentize-go; the -airgap installer's Go module bundles both. No patched Go exists for Intel macOS or Windows-on-ARM64 offline (cosmonic/desktop#433).
metadata:
  version: "1.0.1"
  author: Cosmonic
  upstream: "bytecodealliance/componentize-go v0.4.1 + main @ 20f3b0c; wasmCloud/go component/v0.1.3; golang/go#76775"
---

# Cosmonic Go: componentize-go components on WASI p3

A Go component on Cosmonic Desktop is built by **componentize-go** — never TinyGo,
never standalone `wit-bindgen-go`. It targets the **async WASI p3** worlds
(`wasi:http/handler@0.3.0`, `wasmcloud:nats@0.1.0`), which is what makes the
component warm-pool capable (`poolSize` does something) and lets bodies stream and
goroutines run concurrently. This skill is the toolchain playbook: which versions
work together, how to install them per OS, the two project shapes and their build
commands, and every failure we have hit. `cosmonic-sandbox` owns the scaffold →
build → push → apply loop; `cosmonic-nats` owns the NATS patterns.

**Read in this order:** the hard rules → the versions table → install for your
OS → the project shape you are in → verify → the gotcha catalogue when a build
fails.

## Hard rules

- **componentize-go builds every Go component.** Nothing else emits the
  component-model async ABI: standalone `wit-bindgen-go` v0.7.0 silently generates
  **zero functions** for an async-only package (the build "succeeds" and the
  component exports nothing), and other Go-to-Wasm compilers have no p3 path.
- **Know which project shape you are in** (below). On the **wasmCloud Go SDK**
  (`go-http`): `componentize-go build` only — **never `bindings`**, the SDK ships its
  own and `bindings` rewrites `go.mod`. **WIT-first** (`go-nats-*`, your own WIT):
  `bindings` then `build`, and the `go.mod` rewrite is expected.
- **Always pass `-w <world>`.** componentize-go merges every world it discovers from a
  dependency's `componentize-go.toml`; unpinned, the SDK's default world is the sync p2
  one and you silently get a `wasi:http/incoming-handler` component.
- **Async worlds need the patched Go.** Stock Go has no async support
  ([golang/go#76775], unmerged). componentize-go uses the `go` on PATH when it carries
  the patch and otherwise **downloads** `go1.25.5-wasi-on-idle-v2` (dicej/go) into its
  cache on the first build — network once, then never. The note
  `does not support async operation; will use downloaded version` is expected.
- **Never `go install …/componentize-go@latest` on Windows-on-ARM64.** It installs a
  launcher that bootstraps by downloading `componentize-go-windows-arm64.tar.gz`,
  which does not exist (404) and leaves a shim shadowing any working binary. Use the
  **`windows-amd64`** release archive; it runs under Windows' x64 emulation.
- **On an air-gapped Desktop install, install nothing.** The installer's Go module
  already carries the pinned pair and a seeded module cache; Desktop wires them into
  its builds and into a `componentize-go` wrapper on your PATH (below).
- **Bind a p3 component as `handler`**, not `incoming-handler`; `poolSize` applies.
  `cosmonic_image_inspect` shows which world a built component exports.

## Known-working versions (verified 2026-09-01)

| Piece | Version | Where | Notes |
|---|---|---|---|
| **wasmCloud Go SDK** | `go.wasmcloud.dev/component` **component/v0.1.3** | [wasmCloud/go] | Ships its own bindings (never regenerate them) and a `sleep` package. Carries the `wasmcloud:component-go` worlds' WIT and the `componentize-go.toml` that lets componentize-go find them. component/v0.1.4 (2026-08-31) exists; not yet verified against Desktop's starters. |
| **componentize-go** | **v0.4.1** (release, 2026-07-22) — what Desktop pins and its air-gap Go module bundles | [componentize-go releases] | Also verified: **main @ `20f3b0c`** (2026-08-27, "update to wit-bindgen 0.61.1"); it still reports `0.4.1`. Either way: `build` only on SDK projects, never `bindings`. |
| **wit-bindgen** | **0.61.1** | vendored in componentize-go main @ 20f3b0c | The generator componentize-go carries; confirmed in every generated header. The v0.4.1 release carries an earlier rev, which is fine for the shipped starters. |
| **wit-bindgen-go** (standalone) | v0.7.0 | [bytecodealliance/go-modules] | **Do not use.** Emits zero functions for an async-only package. |
| **`go.bytecodealliance.org/pkg`** | `v0.2.4-0.20260806154504-91f6c4863e67` | module cache, **indirect via the SDK** | Carries `wasihttp` (the `net/http` adapter) and the p3 bindings for `go-http`. The `go-nats-*` starters pin `v0.2.3` directly, and componentize-go v0.4.1's `bindings` rewrites that to `v0.2.2` — both are seeded into the air-gap module cache. |
| **Go toolchain** | **Go 1.25+** on PATH; the **patched** `go1.25.5-wasi-on-idle-v2` for async worlds | [dicej/go releases], auto-installed by componentize-go | The exact tag componentize-go v0.4.1 hardcodes. Stock Go has no async support ([golang/go#76775]). Patched builds exist for macOS arm64, Linux x64/arm64, Windows x64 only. |
| **wac** (optional) | 0.10.1 | Desktop's signed tool bundle (`~/.local/bin`) | Composition only; see `cosmonic-sandbox` `references/composition.md`. |

Bump discipline: the SDK, componentize-go and `pkg` move together. Change one, rebuild
`go-http` and one `go-nats-*` starter, and re-run the verify step before trusting it.

## Install componentize-go

**Lean (online) Desktop installs provision no Go toolchain** — Desktop's Preflight
doctor installs Rust, `wash`, `wkg`, `wasm-tools` and `wac`, and its advisory
`go-component` item only *reports* whether `go` + `componentize-go` resolve (and
whether that Go carries the async patch). You install:

1. **Go 1.25+** from [go.dev/dl] (or your package manager). Any stock Go is fine; the
   async patch is fetched by componentize-go, not by you.
2. **componentize-go**, from the [componentize-go releases] archive for your platform,
   onto your PATH:

| Host | Do this |
|---|---|
| macOS (Apple Silicon) | `componentize-go-darwin-arm64.tar.gz` → `~/.local/bin/componentize-go`, `chmod +x`. |
| Linux x64 / arm64 | `componentize-go-linux-amd64.tar.gz` / `-linux-arm64.tar.gz` → `~/.local/bin/`. glibc-dynamic; RHEL 9 (glibc 2.34) and newer are fine. |
| Windows x64 | `componentize-go-windows-amd64.zip` → a directory on `PATH` (e.g. `%USERPROFILE%\bin`). |
| **Windows on ARM64** | **The same `windows-amd64` zip** — there is no arm64 asset, and it runs under x64 emulation (verified on Windows 11 ARM64, build 10.0.26200: a `go-nats-core-subscriber` built, deployed and served traffic). **Do not `go install`** (its bootstrap 404s and the leftover launcher shadows the real binary). The patched Go componentize-go downloads is also the `windows-amd64` build, also fine under emulation. |
| macOS (Intel) | No patched Go exists, so the **async build is unavailable** (cosmonic/desktop#433). Prefer Rust there. |

`go install github.com/bytecodealliance/componentize-go@latest` works on the other
hosts but leaves a self-bootstrapping launcher that needs network on its first run;
the release archive is a real binary and is what Desktop pins.

Check: `componentize-go --version` prints `0.4.1`; `go version` is ≥ 1.25.

### The air-gapped install: the Go module and its wrapper

The comprehensive `-airgap` installer (built with the Go module switched on, its
default) ships **`go-mirror.tar`**: the patched Go, componentize-go v0.4.1, the seeded
module closure for both `go-http` and `go-nats-*`, and a `manifest.json` naming the
pins. At boot Desktop unpacks it under its state dir, pins every build it runs at it
(`GOROOT`, `GOTOOLCHAIN=local`, `GOPROXY=off`, `GOSUMDB=off`, `GOFLAGS=-mod=readonly`
composed with the starter's own tags, `GOMODCACHE`), and writes a **`componentize-go`
wrapper into `~/.local/bin`** (`componentize-go.cmd` on Windows) that exports the same
offline environment and execs the bundled binary — so `componentize-go build` in the
Builder terminal, or any shell, is offline too.

What the wrapper guarantees, so it never installs over an existing Go setup:

- it is written only onto an **empty slot** or over a previous wrapper of Cosmonic's
  (an ownership marker in its header); a `componentize-go` **you** put in
  `~/.local/bin` is never overwritten or removed — the bundled one still drives
  Desktop's own builds;
- there is deliberately **no `go` shim**: a bare `go` in your shell stays your own Go
  (`~/.local/bin` often precedes Homebrew on PATH and a shim would shadow it
  everywhere);
- it is refreshed when the module changes (an app upgrade), removed when no module is
  staged and by `cosmonicd uninstall --purge`, and only the primary daemon writes it
  (extra host tabs never touch it);
- `COSMONIC_NO_GO_WRAPPER=1` in the daemon's environment opts out, and a fleet can
  build the module with the wrapper off (`manifest.json` `wrapper: false`) so the
  toolchain drives only Desktop's builds.

On such a host: **install nothing**, and the doctor's `go-component` item reads
*Pass* with the pins. Adding dependencies beyond the seeded closure needs
`go mod vendor` — with `GOPROXY=off` nothing new can enter the cache. Offline Go is
available on macOS arm64, Linux x64/arm64 and Windows x64 (the triples with a
patched Go); on Intel Mac and Windows-on-ARM the module is not shipped yet.

## Shape A: the wasmCloud Go SDK (`go-http`)

Ordinary `net/http` on the async p3 `wasi:http/handler@0.3.0`:

```go
package main

import (
    "fmt"
    "net/http"

    "go.bytecodealliance.org/pkg/wasihttp"
    _ "go.wasmcloud.dev/component" // anchors the SDK: its WIT + componentize-go.toml
)

func init() { wasihttp.HandleFunc(handle) } // any http.Handler; a ServeMux for routing

func handle(w http.ResponseWriter, _ *http.Request) { fmt.Fprintln(w, "hello") }

func main() {} // a component is not a CLI; handle runs per request
```

`go.mod` requires `go.wasmcloud.dev/component v0.1.3` and
`go.bytecodealliance.org/pkg v0.2.4-0.20260806154504-91f6c4863e67`; the committed
`go.sum` is the complete closure, so the build needs no `go mod tidy` and no network.
The starter's `.wash/config.yaml` runs, and you can run by hand:

```sh
GOFLAGS=-tags=componentizego_async \
componentize-go -w wasmcloud:component-go/wasip3@0.2.0 build -o build/app.wasm
```

- **`-w wasmcloud:component-go/wasip3@0.2.0`** pins the p3 world. Without it the SDK's
  default (sync p2) world is merged in and you get an `incoming-handler` component.
- **`GOFLAGS=-tags=componentizego_async`** selects wasihttp's async p3 implementation.
  Required: componentize-go v0.4.1 does not emit the tag itself (main does, since its
  PR #71), and without it the build dies with the unhelpful
  `failed to resolve import wasi:http/types@0.2.8` — p2 code compiled against a p3 world.
  Set it in `.wash/config.yaml` `build.env` (works on Windows too, where the build runs
  under `cmd /C`), never as a shell prefix only.
- `componentize-go` is invoked **bare**, not `go tool componentize-go`: the `go tool`
  wrapper downloads a release binary on first use and cannot work air-gapped.
- **Never run `componentize-go bindings` here.** The SDK ships its bindings;
  regenerating rewrites `go.mod` to `module wit_component` and drops your requires.
- Dependencies are ordinary Go: `go get`, `go mod tidy`; on an air-gapped host,
  `go mod vendor` for anything beyond the seeded closure.

## Shape B: WIT-first (`go-nats-*` and your own WIT)

The component's world is in `wit/`, named by `componentize-go.toml`
(`worlds = ["cosmonic:core-subscriber/core-subscriber@0.1.0"]`, `wit_paths = ["wit"]`),
and componentize-go generates the bindings into the module (`module wit_component`):

```sh
componentize-go -d ./wit -w <world> bindings --format && go mod tidy
componentize-go -d ./wit -w <world> build -o app.wasm
```

Here the `go.mod` rewrite is expected: `bindings` sets `module wit_component` and
pins `go.bytecodealliance.org/pkg` to the version its generator wants (v0.2.2 on
componentize-go v0.4.1; the starters pin v0.2.3 and `go mod tidy` reconciles it).
The `go-nats-*` starters' `Makefile` does exactly this (`make build`, `make verify`);
their handlers return `witTypes.Result[witTypes.Unit, string]` on the generated
`wasmcloud_nats_*` packages — `cosmonic-nats` §3 has the shapes. Keep `-w` explicit:
the SDK's `componentize-go.toml`, if it is in your module graph, would merge a world
that mandates a `wasi:http/incoming-handler` export.

Switching a WIT-first project to the SDK (`go.wasmcloud.dev/component` ≥ v0.1.3,
friendlier `nats.Message` types, `error` returns): then it is Shape A — `build` only,
never `bindings`, and vendor the WASI deps.

## Verify the component

A Go build that "succeeds" can still export nothing. Always check:

```sh
wasm-tools component wit app.wasm | grep -E 'export (wasi:http/handler@0\.3|wasmcloud:nats/)'
wasm-tools print app.wasm | grep -qE 'async-lift|task-return' && echo async ABI ok
```

Then `cosmonic_image_inspect` on the pushed image, and bind it as `interfaces: ["handler"]`
(HTTP) or the NATS handler interface. A Go component needs ~2.3 MiB of linear memory
to instantiate — keep the host heap ≥ 4 MiB (`cosmonic-nats-tuning` has the budgets).

## Gotcha catalogue

| Symptom | Cause | Fix |
|---|---|---|
| Generated package has **zero functions**, build exits 0, component exports nothing | standalone `wit-bindgen-go` (v0.7.0) drops every `async func` | build with componentize-go; run the verify step |
| `failed to resolve import wasi:http/types@0.2.8` | `GOFLAGS=-tags=componentizego_async` missing on an SDK (p3) build | set the tag in `.wash/config.yaml` `build.env` |
| Component exports `wasi:http/incoming-handler`, `poolSize` does nothing | no `-w`; the SDK's default p2 world was merged | `-w wasmcloud:component-go/wasip3@0.2.0` |
| `failed to find export of interface 'wasi:http/incoming-handler@0.2.8' function 'handle'` on a NATS component | a dependency's `componentize-go.toml` merged its world | pass `-w <your world>` explicitly |
| `go.mod` now says `module wit_component`, requires gone | `componentize-go bindings` on an SDK project | `git checkout go.mod go.sum`; use `build` only |
| `does not support async operation; will use downloaded version` | the `go` on PATH is stock; componentize-go fetches the patched one | expected; needs network once (never on an air-gapped Desktop: its module has the patch) |
| First build downloads ~55 MB / hangs offline | same download, no network | connect once, or use the air-gapped installer |
| `module lookup disabled by GOPROXY=off` / `go: cannot find module` on an air-gapped host | a dependency outside the seeded closure | `go mod vendor` on a connected machine and commit `vendor/` |
| `go: go.mod requires go >= 1.25` / `GOTOOLCHAIN` download attempt | host Go too old | install Go 1.25+; air-gapped builds pin `GOTOOLCHAIN=local` |
| Windows-on-ARM64: `componentize-go-windows-arm64.tar.gz: 404` | `go install` launcher, no arm64 asset | delete the launcher; use the `windows-amd64` release archive |
| `componentize-go: command not found` in the Builder terminal on an air-gapped host | wrapper not written: daemon not primary, `COSMONIC_NO_GO_WRAPPER`, or module built with `wrapper: false` | `cosmonicd paths`; check `~/.local/bin/componentize-go`; Desktop's own builds still work |
| `go tool componentize-go` fails offline | the `go tool` wrapper downloads a release binary | invoke `componentize-go` bare |
| Handler works once, then stale state / a trap kills in-flight calls | package-level state on a warm instance (`poolSize`), or a panic | treat globals as a cache; keep handlers panic-free |
| Timers/tickers never fire, `time.Sleep` blocks the instance | the Go timer trap on async hosts | use the SDK's `sleep` package; `cosmonic-nats-tuning` error catalogue |

## When to open the other skills

- `cosmonic-sandbox` — the loop: scaffold (`go-http`), `cosmonic_dev_start`, `cosmonic_project_publish`,
  `cosmonic_workload_apply`, the Workload spec (`handler` binding, `allowedHosts`), air-gap.
- `cosmonic-nats` — which `go-nats-*` pattern, the async WIT surface, grants and
  subscriptions; `cosmonic-nats-tuning` for capacity, ack windows and the error catalogue.
- `references/upstream.md` — the upstream projects, what each is for and where its docs
  are, so you can go one level deeper than this file.

[wasmCloud/go]: https://github.com/wasmCloud/go
[componentize-go releases]: https://github.com/bytecodealliance/componentize-go/releases
[bytecodealliance/go-modules]: https://github.com/bytecodealliance/go-modules
[dicej/go releases]: https://github.com/dicej/go/releases
[golang/go#76775]: https://github.com/golang/go/pull/76775
[go.dev/dl]: https://go.dev/dl/
