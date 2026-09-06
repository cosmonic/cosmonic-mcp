# Will this crate build to `wasm32-wasip2`?

No exhaustive list is possible, but most pure-Rust crates work, the failures cluster around a few
root causes, and you can check any single crate in ~30 seconds.

## The 30-second check

```bash
cargo new --lib /tmp/wasmcheck && cd /tmp/wasmcheck
cargo add <crate>            # add the exact feature set you intend to use
cargo build --target wasm32-wasip2 2>&1 | tail -20
```
If it compiles, it builds. If it fails, the error almost always names one of the root causes below.
(Compiling is necessary but not sufficient: a crate can build yet fail at runtime if it calls an
unsupported host API. For the common pure-compute/serialization crates this is rarely an issue.)

## What makes a crate fail (and the usual fix)

| Root cause | Typical crates | Fix / alternative |
|---|---|---|
| Spawns OS threads | `rayon`, thread-pool crates | single-threaded path, or a feature that disables it |
| Opens its own sockets (mio/tokio net) | `tokio` (net/rt-multi-thread), `reqwest` default | use the p3 `wasip3::http::client` (recipes §4) |
| C / system libs | `openssl`, `native-tls`, anything `-sys` | `rustls`-based crate, or a pure-Rust equivalent |
| Ambient randomness | crates pulling old `getrandom` | `getrandom` ≥0.2 has a wasi backend; `wasi:random` via the host |
| Ambient wall-clock at init | some time crates | get time from `SystemTime::now()` (host-provided) |
| `std::fs`/path assumptions | crates touching the filesystem | WASI preopens, or avoid |
| Proc-macro that shells out | rare | usually fine: proc-macros run on the host, not in wasm |

Serialization, parsing, math, codecs, templating, and rendering crates (`serde`/`serde_json`,
`qrcode`, `regex`, `image` decoders, `base64`, etc.) are almost always fine.

## Seed table (verified in Desktop apps)

| Crate | Status | Notes |
|---|---|---|
| `wasip3` | ✅ | the **p3** HTTP surface this skill uses (`handler` inbound + `client` outbound); start here |
| `http`, `http-body-util` | ✅ | build/read requests for p3 outbound (`recipes.md` §4) |
| `serde`, `serde_json` | ✅ | core JSON |
| `urlencoding` | ✅ | query/URL decode |
| `qrcode` | ✅ | use `default-features = false, features = ["svg"]` to drop the `image` dep |
| `wit-bindgen` | ✅ | raw bindings (the `http-kv-handler` template) |
| `wstd` (+ `axum`/`wstd-axum`) | ✅ | the **p2** HTTP surface (upstream `wash` templates); not for the p3 skeleton |
| `chrono` | ⚠️ avoid | unnecessary; use the date helpers in `recipes.md` §5 |
| `tokio` (net/rt) | ❌ | no sockets/threads on wasip2; use the p3 `wasip3::http::client` |
| `reqwest` (default) | ❌ | pulls a native TLS/socket stack; use the p3 `wasip3::http::client` (recipes §4) |

> **Maintainer note:** the seed table above is hand-curated. A plan to auto-generate a top-~200-crate
> compatibility table via CI compile-probing lives in `README.md` → "Roadmap", not here; this file
> stays agent-facing.
