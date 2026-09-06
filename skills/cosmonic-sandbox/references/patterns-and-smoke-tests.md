# Patterns & smoke tests

The verify step (workflow step 7) is where tokens get wasted: dumping full HTML/JSON bodies into
context to "check it works." Don't. Each pattern below pairs the **app shape** with the **minimal
checks** that prove it, written to print a status line or a single field rather than a whole body.

**Jump to:** [curl idioms](#token-efficient-curl-idioms) · [pattern → smoke tests](#pattern--smoke-tests) ·
[deploy-level checks](#deploy-level-checks-any-app) · [reusable smoke script](#reusable-smoke-script)

## Token-efficient curl idioms

```bash
H=http://<NAME>.localhost:8200
# Routing check without DNS (the ingress routes by Host header; also the form for Windows before 10 1709):
#   H=http://127.0.0.1:8200 and add  -H 'Host: <NAME>.localhost'  to each curl.

# status + content-type only, no body:
curl -s -o /dev/null -w '%{http_code} %{content_type}\n' "$H/"

# one field out of a JSON response (needs python3; jq if present):
curl -s "$H/api/thing?x=1" | python3 -c 'import sys,json;print(json.load(sys.stdin)["field"])'

# just the first bytes of a large body (SVG/HTML), to confirm shape:
curl -s "$H/qr.svg?data=hi" | head -c 120
```

Rule of thumb: **never `curl` a full HTML page or a 30-element JSON array into context.** Assert one
status code, one content-type, or one field. Pipe through `head -c`, `python3`, or `grep -o` to clip.

---

## Pattern → smoke tests

### 1. Static page + routes (`/` serves a UI)
What to prove: home returns HTML; unknown routes 404 (not 500).
```bash
curl -s -o /dev/null -w '%{http_code} %{content_type}\n' "$H/"          # 200 text/html
curl -s -o /dev/null -w '%{http_code}\n' "$H/does-not-exist"            # 404
```

### 2. JSON API with a query/path parameter
What to prove: valid input → 200 + the expected field; missing/invalid input → 4xx with a structured
error (proves no panic); correct content-type.
```bash
curl -s -o /dev/null -w '%{http_code} %{content_type}\n' "$H/api/echo?msg=hi"   # 200 application/json
curl -s "$H/api/echo?msg=hi" | python3 -c 'import sys,json;print(json.load(sys.stdin)["message"])'
curl -s -w ' [%{http_code}]\n' "$H/api/echo"                                    # {"error":...} [400]
```

### 3. Customization params actually take effect (e.g. QR colors, sizes)
What to prove: changing a parameter changes the output, not just that *a* response comes back.
```bash
# the requested color must appear in the rendered SVG:
curl -s "$H/qr.svg?data=x&dark=%235eead4" | grep -o 'fill="#5eead4"' | head -1
# an over-limit / bad input is rejected, not silently truncated:
curl -s -o /dev/null -w '%{http_code}\n' "$H/qr.svg?data=$(python3 -c 'print("x"*5000)')"   # 400
```

### 4. Outbound API aggregator (geocode, weather, proxy)
What to prove: happy path returns upstream-derived data; bad input → 4xx (not 500); **egress
allowlist is correct**. The single most common failure here is a missing `allowedHosts` entry;
symptom is every call failing with a gateway/egress error even though the code is right.
```bash
curl -s "$H/api/nights?zip=80301" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get("location") or d.get("error"))'
curl -s -w ' [%{http_code}]\n' "$H/api/nights?zip=abc"                  # {"error":...} [400]
```
If the happy path errors with "upstream"/"failed to reach": check `localResources.allowedHosts`
lists **every** host the code calls (recipes §6b), then re-apply the Workload.

### 5. Persistent state (key-value)
What to prove: a written value survives a read; (optionally) survives a restart if backend is durable.
```bash
curl -s -X POST "$H/kv/foo" -d 'bar'                                    # store
curl -s "$H/kv/foo"                                                     # -> bar
```
In-memory backend (the default) is wiped on restart; only expect persistence if `.wash/config.yaml`
set `wasi_keyvalue_path`/`_nats_url`/`_redis_url`.

### 6. MCP server (`rust-mcp`)
What to prove: `initialize` answers through the ingress with the spec version (proves the Host
guard is right); `tools/list` shows your tools. Full curl forms in `references/mcp-servers.md` §6.
```bash
curl -s -X POST -H 'Host: <NAME>.localhost' http://127.0.0.1:8200/ \
  -H 'Content-Type: application/json' -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2026-07-28' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"check","version":"0"}}}' \
  | grep -o '"protocolVersion":"[^"]*"'                                    # "2026-07-28"
```
A `403` here is `MCP_ALLOWED_HOSTS` not listing the ingress host. A `404` is the wrong hostname.

---

## Deploy-level checks (any app)

```bash
# is it running? (prefer a names/state view over the full spec dump)
# cosmonic_workload_list  ->  look for state: running, restarts: 0
# if a route 502s or never responds, tail logs:
# cosmonic_logs_query            ->  look for panics / "allowedHosts" egress denials
```

A `running` workload that 500s on every request is almost always a **panic** (an `unwrap`/`[]` that
slipped past review) or a **missing `allowedHosts`** entry. Both are visible in `cosmonic_logs_query`.

---

## Reusable smoke script

Run the bundled `scripts/smoke.sh` against a deployed workload (checks home `200` + unknown-route
`404` out of the box; add per-app assertions in the marked section):

```bash
H=http://<name>.localhost:8200 scripts/smoke.sh
# or, without DNS: H=http://127.0.0.1:8200 HOST=<name>.localhost scripts/smoke.sh
```

Copy it next to a project if you want to keep app-specific assertions with the code.
