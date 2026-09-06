# Apps that use a local AI model

Read this when the request mentions Ollama, LM Studio, llama.cpp / `llama-server`, vLLM, "my local
model", "on-device", "private / offline AI", or an OpenAI-compatible endpoint on this machine. The
model server runs on the **host's loopback**; the sandboxed component reaches it through a
deliberately narrow door.

## The door: `allowedHostLoopbackPorts` + `host.wasmcloud.internal`

A component cannot dial `127.0.0.1` — loopback is outside the sandbox. Instead it dials the reserved
name **`host.wasmcloud.internal`** on a port the Workload was granted:

```yaml
spec:
  components:
    - name: <NAME>
      image: oci.localhost:8200/apps/<NAME>:0.1.0
      localResources:
        allowedHosts: ["host.wasmcloud.internal"]     # outbound wasi:http allow-list (deny-all default)
        allowedHostLoopbackPorts: ["11434"]           # "PORT" (TCP) or "PORT/udp"; no ranges, no wildcards
```

| Model server | Default port | Base URL from the component |
|---|---|---|
| Ollama | `11434` | `http://host.wasmcloud.internal:11434/v1` (OpenAI-compatible) or `/api/generate` |
| LM Studio | `1234` | `http://host.wasmcloud.internal:1234/v1` |
| llama.cpp `llama-server` | `8080` | `http://host.wasmcloud.internal:8080/v1` |
| vLLM | `8000` | `http://host.wasmcloud.internal:8000/v1` |

**Two keys open this door, and you hold only one.** The workload grant above is inert until the
host operator also enables loopback grants in **Cosmonic Desktop → Settings → Security** (the daemon's
`PUT /v1/egress allow_host_loopback`, off by default). Say this to the user before promising it
works; a request to `host.wasmcloud.internal` that fails with an egress denial in `cosmonic_logs_query`
while the code is right means the host-side switch is off. Fail-closed on both sides: an empty
`allowedHostLoopbackPorts` denies all.

Every OS uses the same `host.wasmcloud.internal` name and the same two-key rule.

## Calling the model (OpenAI-compatible chat)

Use the outbound idiom from `recipes.md` §4 (`wasip3::http::client::send`, `http` + `http-body-util`
crates). A minimal chat call, panic-free:

```rust
async fn chat(base: &str, model: &str, prompt: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "stream": false
    }).to_string();
    let req = http::Request::builder()
        .method(http::Method::POST)
        .uri(format!("{base}/chat/completions"))
        .header("content-type", "application/json")
        .body(http_body_util::Full::new(bytes::Bytes::from(body)))
        .map_err(|e| e.to_string())?;
    let wasi_req = wasip3::http_compat::http_into_wasi_request(req).map_err(|e| format!("{e:?}"))?;
    let resp = wasip3::http::client::send(wasi_req).await.map_err(|e| format!("{e:?}"))?;
    let resp = wasip3::http_compat::http_from_wasi_response(resp).map_err(|e| format!("{e:?}"))?;
    if !resp.status().is_success() {
        return Err(format!("model server HTTP {}", resp.status().as_u16()));
    }
    let bytes = http_body_util::BodyExt::collect(resp.into_body()).await
        .map_err(|e| format!("{e:?}"))?.to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    v.pointer("/choices/0/message/content").and_then(|c| c.as_str())
        .map(str::to_string).ok_or_else(|| "no choices in response".to_string())
}
```

Read the base URL and model name from env (`localResources.environment.config`:
`MODEL_BASE_URL`, `MODEL_NAME`) so the user can switch servers without a rebuild. Model calls take
seconds: keep `stream: false` for the first version, return the whole answer, and set a generous
client timeout. Do not add an API key for a local server unless the user's server requires one; if
it does, use `secretFrom`, never a literal.

## Smoke test

```bash
# Is the model server up on the host? (host side, not the sandbox)
curl -s http://127.0.0.1:11434/v1/models | head -c 200
# Through the app:
curl -s -H 'Host: <NAME>.localhost' http://127.0.0.1:8200/api/summarize -d 'text=hello' | head -c 200
```

An egress denial for `host.wasmcloud.internal` in `cosmonic_logs_query` → the Settings → Security switch
or `allowedHostLoopbackPorts` is missing; a connection refused → the model server is not running on
that port.

## Not yet: the `cosmonic:llm` host interface

The daemon carries a draft `cosmonic:llm@0.1.0` interface (OpenAI-shaped `inference` + `embeddings`,
proxied by the host to Ollama or any OpenAI-compatible endpoint) that would let a component import
the model as a capability with no egress at all. It is **feature-gated off** in Desktop 0.5.x — do
not build on it or tell the user it is available. Use the loopback door above.

## Running the coding agent itself on a local model

That is a Builder/agent setting (Cosmonic Desktop → Settings → Models, `ANTHROPIC_BASE_URL` for
Claude Code, `~/.pi/agent/models.json` for Pi), not something the workload does. If the user asks
for it, point them at Desktop's Models settings; this skill's job is the app.
