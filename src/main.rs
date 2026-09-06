//! The `cosmonic-mcp` binary: an MCP server over stdio, in front of a running
//! Cosmonic Desktop daemon.
//!
//! Register it with any MCP client:
//!
//! ```json
//! { "mcpServers": { "cosmonic": { "command": "/path/to/cosmonic-mcp" } } }
//! ```
//!
//! It takes no arguments. Everything it needs — the daemon socket — is
//! resolved the same way the daemon resolves it (`COSMONIC_SOCKET`, then
//! `COSMONIC_STATE_DIR`, then the platform default), so a client only ever has
//! to name the binary.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    init_logging();
    // A --version/--help that needed clap would be weight in a binary meant to
    // fit in an .mcpb; there are no other arguments to parse.
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!(
                    "{} {}",
                    cosmonic_mcp::SERVER_NAME,
                    cosmonic_mcp::SERVER_VERSION
                );
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "cosmonic-mcp {}\n\n\
                     A Model Context Protocol server for Cosmonic Desktop. Speaks MCP over\n\
                     stdio and drives the local daemon over its unix socket / named pipe.\n\n\
                     USAGE:\n    cosmonic-mcp\n\n\
                     It takes no arguments. Register it with an MCP client as the `command`.\n\n\
                     ENVIRONMENT:\n    \
                     COSMONIC_SOCKET      explicit daemon endpoint (wins outright)\n    \
                     COSMONIC_STATE_DIR   the socket is found beside this directory\n\n\
                     Logs go to stderr; stdout is the JSON-RPC channel.",
                    cosmonic_mcp::SERVER_VERSION
                );
                return Ok(());
            }
            other => {
                anyhow::bail!("unexpected argument {other:?}; cosmonic-mcp takes none (--help)");
            }
        }
    }
    cosmonic_mcp::serve().await
}

/// Logs go to **stderr**: stdout is the JSON-RPC channel and a single stray
/// line on it corrupts the stream for the whole session.
fn init_logging() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_env("COSMONIC_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(fmt::layer().with_writer(std::io::stderr).with_ansi(false))
        .with(filter)
        .init();
}
