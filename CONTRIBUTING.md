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
