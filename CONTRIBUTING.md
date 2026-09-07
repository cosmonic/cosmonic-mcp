# Contributing

## Where this code lives

This repository is a **mirror**. The crate is developed in
[`cosmonic/desktop`](https://github.com/cosmonic/desktop) at
`daemon/crates/cosmonic-mcp`, where it is built and tested against the daemon it
drives — the two ship together and the MCP surface has a contract test on each
side.

Open issues here; open pull requests against `cosmonic/desktop`.

`scripts/sync-from-desktop.sh` copies `src/`, `skills/` and the `cosmonic-api`
wire types across and stamps the version. It is the only way this repo's source
should change; `--check` makes it a drift tripwire. The `Cargo.toml` files are
deliberately NOT synced — the monorepo's are workspace-relative and this repo's
are standalone — so when a dependency version moves there, move it here by hand
and check that both builds still produce the same `tools/list` on the wire.

## Testing

`cargo test` covers the crate. CI adds a second pass that is deliberately
**not** ours: `tests/conformance_test.py` drives the built binary with the
official **Python MCP SDK**.

The point is independence. `cargo test` and the e2e over in
`cosmonic/desktop` are written against the same assumptions this crate is, so
neither can catch a mistake that is *consistent* with those assumptions — a
schema shape every client but ours rejects, a field the Rust SDK serialises in
a way only the Rust SDK reads back. A different language, SDK and JSON-RPC
stack can.

That is not theoretical: the MCP Inspector found 17 schema-portability
problems here that a hand-rolled driver had passed clean, two of them
introduced by the audit that wrote the driver. The conformance test now pins
that class — remove the `type`-array normalisation and it names all 17 sites.

```console
$ pip install "mcp==2.2.0"
$ cargo build --release --bin cosmonic-mcp
$ python tests/conformance_test.py target/release/cosmonic-mcp
```

It needs no daemon: every assertion is about the protocol surface, which the
server answers whether or not `cosmonicd` is running.

Worth running by hand too, and named by the Connectors Directory checklist:

```console
$ npx @modelcontextprotocol/inspector@latest --cli ./cosmonic-mcp \
      --method tools/list --strict
```

## What must stay true

Two invariants keep this crate shippable, and both are easy to break by accident:

- **It must never depend on `cosmonic-host`.** That would drag the
  wash-runtime / wasmtime build in behind it, and this binary has to stay small
  enough to ship inside an `.mcpb` bundle (~5 MB release today). The crate talks
  to the daemon only over the documented `/v1` HTTP-over-socket API.
- **Annotations are a security control, not metadata.** Every tool carries a
  `title` and exactly one of `readOnlyHint` / `destructiveHint`, because that
  pair is what a client's auto-permission model keys off: a mutation marked
  read-only runs with no confirmation prompt. Tests in `src/server.rs` fail the
  build on a missing or contradictory annotation, on any tool that takes an
  `action`-style selector, and on a description that instructs the model how to
  behave rather than saying what the tool does.

`skills/` is generated — vendored from
[`cosmonic/agent-integrations`](https://github.com/cosmonic/agent-integrations)
by a script in the Desktop repo, along with the `SKILLS` table in
`src/skills.rs`. Never hand-edit either.
