# Recipes: copy-paste skeletons for a Rust HTTP component

Everything here builds clean to `wasm32-wasip2` and matches the conventions used across the existing
Desktop apps. **Copy these instead of reading an existing project to re-derive the idioms.** For a
ready-made starting point, `cp -r assets/skeletons/rust-http <abs>/<project-name>` and edit.

Jump to: [project files](#1-project-files) · [strict lints](#2-the-strict-lints-read-before-writing-code)
· [HTTP skeleton](#3-minimal-http-server-skeleton) · [outbound HTTP](#4-calling-an-external-api-outbound-http)
· [date math](#5-date-math-no-chrono) · [Workload manifests](#6-the-two-canonical-workload-manifests)
· [MCP](#7-an-mcp-server-is-not-a-recipe-here) · [crates](#8-crates-that-build-to-wasm32-wasip2)

---

## 1. Project files

A Path-B Rust HTTP project is these files (the bundled `assets/skeletons/rust-http/` shape; a
`Cargo.lock` is committed for reproducible/offline builds):

```
<project>/
  Cargo.toml
  Cargo.lock
  .wash/config.yaml
  src/lib.rs
  src/index.html        # only if you serve a UI
```

**`Cargo.toml`**: the canonical strict shape (the `[workspace.lints]` block is load-bearing; see §2).
This is a WASI **Preview 3** component — it exports `wasi:http/handler@0.3.0` via the `wasip3` crate,
which is warm-pool capable (see `crds.md` → "Warm instance pool"):

```toml
[package]
name = "my-app"          # crate name; underscores in the .wasm filename
edition = "2021"
version = "0.1.0"
rust-version = "1.94"    # p3 build floor; see the wasm-component-ld note below

[lints]
workspace = true

[workspace]

[workspace.lints.rust]
warnings = "deny"
unsafe_code = "deny"

[workspace.lints.clippy]
unwrap_used = 'deny'
expect_used = 'deny'
panic = 'deny'
indexing_slicing = 'deny'

[lib]
crate-type = ["cdylib"]

[dependencies]
# p3 (wasi:http/handler@0.3.0). The wasip3 crate carries the WIT (no vendored wit/ dir).
wasip3 = { version = "0.7.0", features = ["http-compat"] }
wit-bindgen = { version = "0.57.1", default-features = false, features = ["async", "async-spawn", "inter-task-wakeup"] }
http-body = "1"
bytes = "1"
# serde_json = "1"      # add for JSON
# urlencoding = "2"     # add for query/URL decoding
# http = "1"            # add for outbound HTTP (§4)
# http-body-util = "0.1"# add for outbound HTTP (§4)

[profile.release]       # smaller artifacts
lto = true
opt-level = "s"
strip = true
```

**`.wash/config.yaml`**: note `component_path` uses the crate name with **underscores**:

```yaml
build:
  command: cargo build --target wasm32-wasip2 --release
  component_path: target/wasm32-wasip2/release/my_app.wasm
```

**No `wit/` dir**: the `wasip3` crate carries the `wasi:http@0.3.0` WIT, and the daemon infers
`interfaces: ["handler"]` from the export. (Capability shapes that hand-write `wit_bindgen` bindings
against a p2 interface — e.g. `http-kv-handler` — do vendor a `wit/world.wit`; see `templates.md`.)

### Build toolchain: Rust 1.94+ for p3

p3 components need **Rust 1.94 or newer**. The `wasip3` 0.7.x crate embeds a
component-type section that only `wasm-component-ld` 0.5.20+ can process, and that
linker ships bundled with Rust 1.94. On an older toolchain the build fails in the
`wasm-component-ld` link step with a component encode/decode error (the exact
message varies by toolchain version). The `rust-version = "1.94"` in the
`Cargo.toml` above turns that into a clear `requires rustc 1.94` instead. rustc
uses its *bundled* `wasm-component-ld`, so a newer standalone one on your PATH
(the runtime `wasmtime` shadow below is a separate thing) does not help here.
Cosmonic Desktop's managed toolchain already satisfies this, so it only bites a
hand-rolled older toolchain. Re-check the floor if you bump `wasip3`/`wit-bindgen`.

### Running a p3 component with the standalone `wasmtime` CLI (optional)

You don't need `wasmtime` to build or deploy. Cosmonic Desktop's host embeds it (currently 47.0.3)
and runs p3 components for you, so `cosmonic_dev_start` / `cosmonic_project_publish` is the normal loop. If you
want to run a component directly, outside Cosmonic, two things matter.

**Version: wasmtime 46 or newer.** p3's `wasi:http/handler@0.3.0` uses component-model async, so 45
and older fail with `` `stream` requires the component model async feature``. Validated against this
template: 46.0.x, 47.0.x, and 48.0.x all serve it; 45 and below do not. Matching the host's 47.x is
the safest choice.

**Flags: enable cli, p3, and http, or the errors look like a version problem when they aren't.** A
missing `-S p3` surfaces the async-feature error above; a missing `-S cli` surfaces
`get-environment has the wrong type`, because the component imports `wasi:cli/environment`.

```bash
wasmtime serve -S cli -S p3 -S http <component>.wasm
curl http://127.0.0.1:8080/
```

**PATH shadow.** A `wasmtime` under `~/.cargo/bin` (from `cargo install`) or an old Homebrew build
often shadows a newer install and can sit well below 46. Run `wasmtime --version` and
`which -a wasmtime` before blaming the component.

---

## 2. The strict lints (read before writing code)

The `Cargo.toml` above **denies** `unwrap`, `expect`, `panic`, and `indexing_slicing`, and treats all
warnings as errors. This is deliberate: a panicking component returns a 500 with no useful message.
Write to satisfy these from the start. Note **`wash build` runs `cargo build`, which enforces only
the `warnings = "deny"` (rustc) group — it does *not* evaluate the clippy lints**; run `cargo clippy
--target wasm32-wasip2` to catch `unwrap`/`expect`/`panic`/`indexing_slicing` (a stray `.unwrap()`
compiles fine under `wash build` and only faults at runtime). Follow the table so it stays clean
under both:

| Don't | Do |
|---|---|
| `vec[i]`, `&s[a..b]` | `vec.get(i)` / `s.get(a..b)` (returns `Option`), or iterate |
| `opt.unwrap()`, `res.unwrap()` | `let Some(x) = opt else { return ... }`, `?`, `.unwrap_or(default)`, `.map_or(...)` |
| `.expect("msg")` | same as above |
| `panic!`, `todo!`, `unreachable!` | return an error `Response` / `Result::Err` |
| array index in a loop | `.iter().enumerate()`, `.zip()`, `.find_map()` |

`a.get(i)` on a `Vec`/slice is a **method** returning `Option`, which is allowed; only the `[]`
operator is denied. `serde_json::Value::get("key")` is likewise fine.

---

## 3. Minimal HTTP server skeleton

Routing, a static page, and a reusable `respond()` helper. This is the 80% case, and it is exactly
`assets/skeletons/rust-http/src/lib.rs`. In p3 the response body is a **stream**, so `respond` hands
the head back to the host and writes the body from a spawned task; it never panics.

```rust
use wasip3::http::types::{ErrorCode, Fields, Request, Response};
use wasip3::http_compat::{http_from_wasi_request, BodyWriter};

struct Component;
wasip3::http::service::export!(Component);

impl wasip3::exports::http::handler::Guest for Component {
    async fn handle(request: Request) -> Result<Response, ErrorCode> {
        // Read only the URI (path + query); take owned copies, then drop the incoming request.
        let request = http_from_wasi_request(request)?;
        let path = request.uri().path().to_string();
        let query = request.uri().query().unwrap_or("").to_string();
        drop(request);
        match path.as_str() {
            "/" => respond(200, "text/html; charset=utf-8", INDEX_HTML.as_bytes().to_vec()),
            "/api/hello" => respond(200, "text/plain; charset=utf-8", b"hello\n".to_vec()),
            _ => respond(404, "text/plain; charset=utf-8", b"not found\n".to_vec()),
        }
    }
}

/// Build a complete p3 response. Never panics.
fn respond(status: u16, content_type: &str, body: Vec<u8>) -> Result<Response, ErrorCode> {
    let headers = Fields::from_list(&[(
        "content-type".to_string(), content_type.as_bytes().to_vec(),
    )])
    .map_err(|e| ErrorCode::InternalError(Some(format!("invalid headers: {e}"))))?;
    let (mut writer, body_rx, result_rx) = BodyWriter::new();
    let (response, _transmit) = Response::new(headers, Some(body_rx), result_rx);
    response.set_status_code(status)
        .map_err(|()| ErrorCode::InternalError(Some("invalid status code".into())))?;
    wasip3::wit_bindgen::spawn(async move {
        let frame = http_body::Frame::data(bytes::Bytes::from(body));
        let _ = writer.send_frame(frame).await;
        drop(writer.stream_writer);            // close the body stream; no trailers
        let _ = writer.result_writer.write(Ok(None)).await;
    });
    Ok(response)
}

const INDEX_HTML: &str = include_str!("index.html");   // drop the const/include if no UI
```

**JSON** (needs `serde_json`): build the string and pass its bytes to `respond`:
```rust
let body = serde_json::json!({ "error": "missing ?msg=" }).to_string();
return respond(400, "application/json", body.into_bytes());
```

**Read a query parameter** (URL-decoded; needs `urlencoding`):
```rust
fn query_param(query: &str, key: &str) -> Option<String> {
    for pair in query.split('&') {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next()?;
        if k == key {
            let raw = parts.next().unwrap_or("");
            return Some(
                urlencoding::decode(raw).map(|c| c.into_owned()).unwrap_or_else(|_| raw.to_string()),
            );
        }
    }
    None
}
// usage inside handle(): let name = query_param(&query, "name");
```

---

## 4. Calling an external API (outbound HTTP)

Two things are required and easy to forget:

1. List every host you call in **`localResources.allowedHosts`**: egress is **deny-by-default**. On
   Cosmonic Desktop the outbound `wasi:http` interface is provided **implicitly** — you do *not* add
   it to `hostInterfaces` (§6b), you only allow the hosts.
2. Add `http = "1"` and `http-body-util = "0.1"` to `Cargo.toml`, and call the p3 `client`. (These
   aren't in the base skeleton's `Cargo.lock`, so an **offline** build needs them cached/vendored too
   — see "Air-gapped installations" in `SKILL.md`.)

Outbound in p3 is `wasip3::http::client::send`; build the request with the `http` crate and read the
response body with `http_body_util::BodyExt`. This adds a `wasi:http/client@0.3.0` import.

```rust
use bytes::Bytes;
use http_body_util::{BodyExt, Empty};
use wasip3::http_compat::{http_from_wasi_response, http_into_wasi_request};

/// GET `url` and parse the body as JSON. Returns a string error instead of panicking.
async fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let req = http::Request::builder()
        .method(http::Method::GET)
        .uri(url)
        .body(Empty::<Bytes>::new())
        .map_err(|e| e.to_string())?;
    let wasi_req = http_into_wasi_request(req).map_err(|e| format!("{e:?}"))?;
    let wasi_resp = wasip3::http::client::send(wasi_req).await.map_err(|e| format!("{e:?}"))?;
    let resp = http_from_wasi_response(wasi_resp).map_err(|e| format!("{e:?}"))?;
    if !resp.status().is_success() {
        return Err(format!("upstream HTTP {}", resp.status().as_u16()));
    }
    let bytes = resp.into_body().collect().await.map_err(|e| format!("{e:?}"))?.to_bytes();
    serde_json::from_slice(bytes.as_ref()).map_err(|e| e.to_string())
}

// Inside handle(), map the String error into a Response (there is no `?` shortcut — get_json
// returns Result<_, String> and handle returns Result<_, ErrorCode>):
//   match get_json("https://api.example.com/v1/thing").await {
//       Ok(v)  => respond(200, "application/json", v.to_string().into_bytes()),
//       Err(e) => respond(502, "text/plain; charset=utf-8", e.into_bytes()),
//   }
```

To POST a body, swap `Empty` for `http_body_util::Full::new(Bytes::from(payload))` and set the
method/headers on the builder.

**Common free, no-key APIs** (and the exact `allowedHosts` entry):

| Need | URL pattern | `allowedHosts` entry | Notes |
|---|---|---|---|
| US ZIP → lat/lon | `https://api.zippopotam.us/us/{zip}` | `api.zippopotam.us` | lat/lon come back as **strings**, so `.parse::<f64>()` |
| City → lat/lon | `https://geocoding-api.open-meteo.com/v1/search?name={q}&count=1` | `geocoding-api.open-meteo.com` | results under `.results[0]` |
| Weather forecast | `https://api.open-meteo.com/v1/forecast?latitude=&longitude=&hourly=...` | `api.open-meteo.com` | forecast horizon ≈ **16 days**; `timezone=auto` returns local times |

(`allowedHosts` is hostname-only — no scheme, no path; see `crds.md`.)

---

## 5. Date math (no `chrono`)

`chrono` is heavy and unnecessary. For calendar arithmetic use Howard Hinnant's algorithms (copy this
into `src/` only when you need it (with `warnings = "deny"`, don't carry unused code):

```rust
/// Days since the Unix epoch (1970-01-01) for a Gregorian date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
/// Inverse -> (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
```
Get "today" from `std::time::SystemTime::now()` → `duration_since(UNIX_EPOCH)` → `secs.div_euclid(86400)`.

---

## 6. The two canonical Workload manifests

`cosmonic_workload_apply` takes a **flat `Workload` JSON object**: top-level `spec.components` and
`spec.hostInterfaces`. It does **not** accept the `spec.template.spec` nesting of a
`WorkloadDeployment` (that fails with `missing field 'components'`). The host naming rule:

> `config.host: <name>.localhost`  →  reachable at  `http://<name>.localhost:8200/`
> (verify routing without DNS with `curl -H 'Host: <name>.localhost' http://127.0.0.1:8200/`).
> Report the URL from the `config.host` you actually applied — see the hostname rule in `SKILL.md`.

Local unsigned images need the `desktop.cosmonic.com/unsafe-allow-unsigned: "true"` annotation.

The `assets/skeletons/rust-http/` starter ships this manifest ready to apply at
`deploy/deploy.yaml` (a §6a-style no-egress `Workload` in YAML, with an explicit empty
`allowedHosts`) — rename the component and apply it, no hand-authoring needed.

### a) No egress (self-contained: serves HTML / computes locally)

```json
{
  "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
  "kind": "Workload",
  "metadata": {
    "name": "<NAME>",
    "namespace": "default",
    "annotations": { "desktop.cosmonic.com/unsafe-allow-unsigned": "true" }
  },
  "spec": {
    "components": [
      { "name": "<NAME>", "image": "oci.localhost:8200/apps/<NAME>:0.1.0", "poolSize": 8 }
    ],
    "hostInterfaces": [
      {
        "namespace": "wasi", "package": "http", "interfaces": ["handler"],
        "config": { "host": "<NAME>.localhost" }
      }
    ]
  }
}
```

`interfaces: ["handler"]` is p3 (the skeleton and the `rust-http`/`rust-mcp` templates, and now
`go-http`); a p2 `ts-http` or upstream `wash` component uses `["incoming-handler"]` and should drop `poolSize`.

### b) With egress (calls external APIs)

Adds the `allowedHosts` allowlist. Egress is deny-by-default; an absent/empty list blocks all
outbound calls. On Cosmonic Desktop the outbound `wasi:http` interface is provided **implicitly** —
you do *not* declare it in `hostInterfaces`, you only list the hosts.

```json
{
  "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
  "kind": "Workload",
  "metadata": {
    "name": "<NAME>",
    "namespace": "default",
    "annotations": { "desktop.cosmonic.com/unsafe-allow-unsigned": "true" }
  },
  "spec": {
    "components": [
      {
        "name": "<NAME>",
        "image": "oci.localhost:8200/apps/<NAME>:0.1.0",
        "localResources": {
          "allowedHosts": ["api.open-meteo.com", "api.zippopotam.us"]
        },
        "poolSize": 8
      }
    ],
    "hostInterfaces": [
      {
        "namespace": "wasi", "package": "http", "interfaces": ["handler"],
        "config": { "host": "<NAME>.localhost" }
      }
    ]
  }
}
```

(`allowedHosts` is hostname-only, no scheme/path; matched case-insensitively. There is **no**
`outgoing-handler` entry in `hostInterfaces` — outbound is implicit, gated by `allowedHosts`.)

> **Warm pool (performance).** These manifests bind the p3 `handler` (what the skeleton and the
> `rust-http`/`rust-mcp` templates export), so they are warm-pool capable — set `poolSize` for
> throughput (a static hello-world goes ~19k → ~57k req/s from `poolSize` `0` → `128`; add
> `maxConcurrency` for an I/O-bound guest). A **p2** component (`ts-http`, or an upstream `wash` template)
> exports `incoming-handler` and silently ignores `poolSize`. See `crds.md` → "Warm instance pool".

---

## 7. An MCP server is not a recipe here

The MCP-server shapes (pure compute, outbound API, desktop-app bridge) live in
`references/mcp-servers.md` with their own manifest; the `rust-mcp` template is the starting point,
not this skeleton.

## 8. Crates that build to `wasm32-wasip2`

`wasm32-wasip2` has no threads, no direct sockets/filesystem outside WASI, and no
`Date::now`/`Math::random`-style ambient calls. Pure-compute and serialization crates work; anything
that opens its own sockets or spawns threads usually does not (use the WASI equivalent — e.g.
`wasip3::http::client` for outbound HTTP, §4).

**Verified working in Desktop apps:** `wasip3` (the p3 HTTP surface this skill uses), `serde` /
`serde_json`, `urlencoding`, `qrcode` (`default-features = false, features = ["svg"]`),
`http-body-util` + `http` (for §4 outbound), `wit-bindgen`, and — for **p2** components — `wstd` and
`axum` + `wstd-axum` (from the `http-handler` template).

For anything else, don't guess; see `references/wasm-crate-compat.md` for how to check a crate in
~30 seconds, and the pre-checked top-crates table.
