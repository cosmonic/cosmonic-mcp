# cosmonic-mcp

The [Model Context Protocol](https://modelcontextprotocol.io) server for
[Cosmonic Desktop](https://cosmonic.com). It lets a coding agent scaffold,
build, deploy and observe WebAssembly component workloads in the local
sandbox — from *"build me an API"* to a running URL.

It is a **stateless facade** over the daemon's `/v1` API, reached across the
local unix socket (named pipe on Windows). It opens no network listener and
needs no auth token: the peer-checked socket is the trust boundary.

```
[Claude Code / Claude Desktop / Codex / Cursor / …]
      │  stdio (JSON-RPC 2.0, MCP)
[cosmonic-mcp]            ← this crate
      │  HTTP/JSON over the unix socket (peer-cred, 0700/0600)
[cosmonicd]               ← Cosmonic Desktop's daemon
```

## Install

`cosmonic-mcp` needs a running Cosmonic Desktop. Register it with any MCP
client as the `command` — it takes no arguments:

```json
{ "mcpServers": { "cosmonic": { "command": "/path/to/cosmonic-mcp" } } }
```

```console
$ claude mcp add --scope user cosmonic -- /path/to/cosmonic-mcp
$ codex mcp add cosmonic -- /path/to/cosmonic-mcp
```

Cosmonic Desktop's **Settings → MCP Server** writes this for you, per client.

## Two binaries, one implementation

This crate builds the standalone `cosmonic-mcp` binary **and** backs
`cosmonicd mcp serve`, which delegates to the same `serve()`. Neither has its
own copy of a tool, a description, or an annotation — an annotation is what a
client's auto-permission model keys off, and two surfaces that could disagree
about which tools are read-only is a security bug waiting to be written.

## Tools

33 tools in ten resource families, named `cosmonic_<resource>_<verb>` so a
family clusters in an alphabetical tool list. Reads and writes are **separate
tools** — no tool takes an `action` selector — and every tool carries a `title`
plus exactly one of `readOnlyHint` / `destructiveHint`.

| Family | Tools |
|---|---|
| `workload_` | `list` `get` `apply` `validate` `start` `stop` `restart` `delete` `draft` `credentials_test` `revision_list` `rollback` |
| `dev_` | `start` `stop` `status` `logs` |
| `project_` | `list` `create` `publish` `delete` |
| `image_` | `inspect` `list` `prune` |
| `secret_` | `set` `list` `delete` |
| `config_` | `list` `set` |
| `registry_` | `list` `test` |
| — | `template_list` · `host_status` · `logs_query` |

Reads: `host_status`, `template_list`, `project_list`, `workload_list`,
`workload_get`, `workload_validate`, `workload_revision_list`,
`workload_credentials_test`, `workload_draft`, `dev_status`, `dev_logs`,
`logs_query`, `image_inspect`, `image_list`, `secret_list`, `config_list`,
`registry_list`, `registry_test`. Everything else writes.

Every tool returns a uniform envelope — `{ status, result, next_steps[],
errors[].recovery }` — so an agent always knows what to do next and how to
recover.

`project_publish`, `workload_delete`, `workload_rollback`, `secret_delete`,
`project_delete` and `image_prune` additionally require `confirm=true`.

**`workload_validate` is the rehearsal**: it reports what an apply would say —
schema errors, deny-all egress, inert loopback ports, secret references this
host does not have — while storing nothing, pulling nothing and scheduling
nothing.

**`project_publish` schedules nothing.** It builds and pushes the component and
returns the digest-pinned image plus a `workloadManifest` to review; applying
that manifest is what deploys it.

## Skills over MCP

The tools are the hands; a **skill** is the manual. This server publishes the
`cosmonic-sandbox` skill family as `skill://` resources
(`io.modelcontextprotocol/skills`), so **connecting is enough** — an agent
gets the playbook without installing anything, including agents with no
skills directory at all.

Disclosure is progressive:

| Tier | URI | Size | Read when |
|---|---|---|---|
| 1 | `skill://index.json` | ~8 KB | Once, at session start |
| 2 | `skill://<name>/SKILL.md` | 14–25 KB | The catalog description matches the task |
| 3 | `skill://<name>/references/<file>.md` | 3–26 KB | The playbook points at it |

Reading everything is 269 KB; reading the index is 8 KB. That gap is the point.

Skill content is embedded at compile time, so the binary is self-contained,
works air-gapped, and cannot ship a playbook documenting a tool it does not
have. `skills::read` resolves a URI by matching it **verbatim** against a
static table — no filesystem, so no path traversal.

## Resources

- `cosmonic://schema/workload` — the `runtime.wasmcloud.dev/v1alpha1` Workload
  spec plus a worked HTTP-API example. Read it before authoring a workload.
- `cosmonic://host`, `cosmonic://workloads`, `cosmonic://templates` — live
  state, fetched at read time.
- `skill://…` — see above.

## Environment

| | |
|---|---|
| `COSMONIC_SOCKET` | Explicit daemon endpoint; wins outright. |
| `COSMONIC_STATE_DIR` | The socket is found beside this directory. |
| `COSMONIC_LOG` | Log filter (`tracing` syntax). Logs go to **stderr** — stdout is the JSON-RPC channel. |

## Security

- **No new network surface** — stdio to the client, a local socket to the daemon.
- **Secrets stay references.** `cosmonic_secret_set` registers a reference (OS keychain, `env://`, 1Password, AWS Secrets Manager);
  values are never returned, logged, or included in an error. A workload whose
  `secretFrom` ref is missing is accepted and parked, and the result tells the
  agent what is missing and that the *user* enters it in Cosmonic Desktop.
- **Egress is fail-closed** — a workload's `allowedHosts` is deny-all until set.
- **Annotations are a control, not metadata.** A tool wrongly marked read-only
  runs without a confirmation prompt, so the crate's tests fail the build on a
  missing, contradictory, or multiplexed annotation.
- **Direct URLs reach less far for an agent than for a person** — the daemon
  refuses cloud instance-metadata endpoints outright, and refuses
  loopback/private targets for MCP-originated requests.

## Build

```console
$ cargo build --release --bin cosmonic-mcp
$ cargo test
```

This is a self-contained workspace: the server, plus the `cosmonic-api` wire
types it speaks, vendored under `crates/`. Nothing else is needed to build it.

No `cosmonic-host` dependency, by design: that would pull the whole
wash-runtime/wasmtime build into a binary meant to stay small enough to ship
inside an [MCP Bundle](https://github.com/modelcontextprotocol/mcpb) — the
bundle is built in
[`cosmonic/claude-mcpb`](https://github.com/cosmonic/claude-mcpb).

Source is synced from the monorepo where it is developed:

```console
$ ./scripts/sync-from-desktop.sh ../desktop
$ ./scripts/sync-from-desktop.sh ../desktop --check   # CI: fail on drift
```

## License

Apache-2.0. See [LICENSE](./LICENSE).
