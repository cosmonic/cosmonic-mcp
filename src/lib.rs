//! `cosmonic-mcp` — the Model Context Protocol server for Cosmonic Desktop.
//!
//! A **stateless facade** over the daemon's `/v1` API, reached across the local
//! unix socket (named pipe on Windows). It adds no network surface and needs no
//! auth token: the peer-checked socket is the trust boundary. See
//! `docs/MCP.md`.
//!
//! ```text
//! [Claude Code / Claude Desktop / Codex / …]
//!       │  stdio (JSON-RPC 2.0, MCP)
//! [cosmonic-mcp]            ← this crate
//!       │  HTTP/JSON over the unix socket (peer-cred, 0700/0600)
//! [cosmonicd]               ← the scheduling brain
//! ```
//!
//! ## One implementation, two binaries
//!
//! `serve()` is the whole server. Two things call it:
//!
//! * the **`cosmonic-mcp`** binary in this crate — what ships standalone and
//!   inside the `.mcpb` desktop-extension bundle;
//! * **`cosmonicd mcp serve`**, which delegates here so every client already
//!   registered against the daemon keeps working unchanged.
//!
//! Neither has its own copy of a tool, a description, or an annotation, which
//! is the point: an annotation is what a client's auto-permission model keys
//! off, and two surfaces that could disagree about which tools are read-only
//! is a security bug waiting to be written.
//!
//! ## What is deliberately absent
//!
//! No `cosmonic-host` dependency — no `wash_runtime`, no `wasmtime`, no
//! reconciler, no secrets backend. This crate carries the tool surface and a
//! socket client, nothing else. That is what keeps the binary small enough to
//! bundle and self-contained enough to open-source.

mod client;
pub mod paths;
mod prompts;
mod resources;
mod server;
pub mod skills;
mod tools;

pub use client::{DaemonClient, DaemonError};
pub use server::{serve, CosmonicMcp};

/// The name this server reports at `initialize`, and the name a bug report
/// should carry. Not `rmcp`, which is what the SDK's default reports.
pub const SERVER_NAME: &str = "cosmonic-desktop";

/// This build's version.
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
