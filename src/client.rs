//! A minimal HTTP/1 client over the daemon's control endpoint — the unix
//! socket, or the named pipe on Windows — used by the MCP server to drive the
//! same `/v1/...` API the Electron app uses. One short-lived connection per
//! request (the API is request/response; no streaming here).
//!
//! This deliberately mirrors `src/main/daemon.js`: connect to
//! `paths::socket_path()`, speak plain HTTP/JSON, no auth token (the
//! peer-checked socket/pipe is the trust boundary).

use std::path::PathBuf;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use serde_json::Value;

/// What went wrong talking to the daemon. The MCP tool layer turns these into
/// agent-readable error envelopes with recovery hints.
#[derive(Debug)]
pub enum DaemonError {
    /// Could not reach the socket at all (daemon not running / wrong path).
    Unreachable(String),
    /// The daemon returned a non-2xx status. Carries the parsed `ApiError`
    /// `code`/`message` when present, else the raw body.
    Api {
        status: u16,
        code: String,
        message: String,
    },
    /// Transport/parse failure after a connection was made.
    Transport(String),
}

impl std::fmt::Display for DaemonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaemonError::Unreachable(m) => write!(f, "daemon unreachable: {m}"),
            DaemonError::Api {
                status,
                code,
                message,
            } => {
                write!(f, "daemon error {status} ({code}): {message}")
            }
            DaemonError::Transport(m) => write!(f, "transport error: {m}"),
        }
    }
}

/// A handle to the local daemon over its unix socket.
#[derive(Clone)]
pub struct DaemonClient {
    socket: PathBuf,
    /// The `x-cosmonic-origin` tag every request carries. The daemon reads it
    /// for deployment-origin bookkeeping on apply and records it in the audit
    /// trail on `POST /v1/shutdown` — so a CLI caller must not masquerade as
    /// the MCP server (a SIEM asking "did an AI agent stop the daemon?" gets
    /// the wrong answer).
    origin: &'static str,
}

impl DaemonClient {
    /// Resolve the daemon socket the same way the daemon itself does.
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            socket: Self::resolved_socket()?,
            origin: cosmonic_api::workload::SOURCE_MCP,
        })
    }

    /// Same resolution, tagged as a direct CLI invocation (`cosmonicd stop`)
    /// rather than the MCP server.
    pub fn new_cli() -> anyhow::Result<Self> {
        Ok(Self {
            socket: Self::resolved_socket()?,
            origin: cosmonic_api::workload::SOURCE_CLI,
        })
    }

    /// The resolved socket path, rejected up front when it is over the
    /// `sockaddr_un` limit. A path that long can never connect; without this
    /// it surfaces as `daemon unreachable: path must be shorter than SUN_LEN`
    /// on every call, which reads as "the daemon isn't running".
    fn resolved_socket() -> anyhow::Result<PathBuf> {
        let socket = crate::paths::socket_path()?;
        crate::paths::check_socket_path(&socket)?;
        Ok(socket)
    }

    /// Test-only: a client pinned to an explicit endpoint (a test socket/pipe)
    /// instead of the resolved default.
    ///
    /// `windows` as well as `test`, because its only callers are the named-pipe
    /// round-trip tests in `api.rs`, which live inside a `#[cfg(windows)]`
    /// module. Gated on `test` alone it is dead code on every other target —
    /// which is what has been failing `cargo clippy -D warnings` on the Linux
    /// CI leg since the pipe transport landed (488eb63).
    #[cfg(windows)]
    pub fn for_endpoint(socket: PathBuf) -> Self {
        Self {
            socket,
            origin: cosmonic_api::workload::SOURCE_MCP,
        }
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket
    }

    pub async fn get(&self, path: &str) -> Result<Value, DaemonError> {
        self.request("GET", path, None).await
    }

    pub async fn post(&self, path: &str, body: Value) -> Result<Value, DaemonError> {
        self.request("POST", path, Some(body)).await
    }

    pub async fn delete(&self, path: &str) -> Result<Value, DaemonError> {
        self.request("DELETE", path, None).await
    }

    /// Issue one request over a fresh connection and return the parsed JSON body
    /// (or `Value::Null` for an empty 2xx). Non-2xx becomes [`DaemonError::Api`].
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, DaemonError> {
        let stream = connect(&self.socket)
            .await
            .map_err(|e| DaemonError::Unreachable(e.to_string()))?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
            .await
            .map_err(|e| DaemonError::Transport(e.to_string()))?;
        // Drive the connection in the background; it completes when the response
        // is fully read and the (non-keep-alive) connection closes.
        tokio::spawn(async move {
            let _ = conn.await;
        });

        let body_bytes = match &body {
            Some(v) => Bytes::from(serde_json::to_vec(v).expect("serialize request body")),
            None => Bytes::new(),
        };
        let mut req = hyper::Request::builder()
            .method(method)
            .uri(path)
            // The unix socket has no host, but HTTP/1.1 requires a Host header.
            .header(hyper::header::HOST, "cosmonicd.local")
            // Tag this surface so the daemon records MCP-driven deploys in the
            // deployment-origin mix (usage analytics; also audited on
            // /v1/shutdown). Read only on apply/shutdown.
            .header("x-cosmonic-origin", self.origin);
        // Tag the current MCP tool (set by call_tool) so the daemon counts tool
        // usage (`mcp.tool` events). Absent outside a tool dispatch.
        if let Ok(tool) = crate::server::CURRENT_TOOL.try_with(|t| t.clone()) {
            req = req.header("x-cosmonic-mcp-tool", tool);
        }
        if body.is_some() {
            req = req.header(hyper::header::CONTENT_TYPE, "application/json");
        }
        let req = req
            .body(Full::new(body_bytes))
            .map_err(|e| DaemonError::Transport(e.to_string()))?;

        let resp = sender
            .send_request(req)
            .await
            .map_err(|e| DaemonError::Transport(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .into_body()
            .collect()
            .await
            .map_err(|e| DaemonError::Transport(e.to_string()))?
            .to_bytes();

        if (200..300).contains(&status) {
            if bytes.is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_slice(&bytes).map_err(|e| DaemonError::Transport(e.to_string()))
        } else {
            // Try to parse the daemon's `ApiError { code, message }`.
            let (code, message) = serde_json::from_slice::<Value>(&bytes)
                .ok()
                .and_then(|v| {
                    // A code with no message, or a message with no code, is
                    // still better than a synthesized one. Only a body that
                    // carries NEITHER falls through to `route_error`.
                    let code = v.get("code").and_then(|c| c.as_str());
                    let message = v.get("message").and_then(|m| m.as_str());
                    match (code, message) {
                        (None, None) => None,
                        (code, message) => Some((
                            code.unwrap_or("error").to_string(),
                            message.unwrap_or_default().to_string(),
                        )),
                    }
                })
                .unwrap_or_else(|| route_error(status, method, path, &bytes));
            Err(DaemonError::Api {
                status,
                code,
                message,
            })
        }
    }
}

/// Describe a daemon response the daemon itself did not describe.
///
/// axum answers an unrouted path with a bare 404 and a wrong-method one with a
/// bare 405 — **no body at all**. The old fallback took `String::from_utf8_lossy`
/// of those zero bytes, so the tool layer formatted `"{message} (HTTP {status})"`
/// into the literal `" (HTTP 405)"`: a leading space and a number. That is the
/// contentless-error pattern the Connectors Directory rejects outright, and it
/// is also the shape a version-skewed pair produces — a new `cosmonicd mcp serve`
/// spawned by a client against an older running daemon that lacks the route
/// (issue #501 B-5).
///
/// So: never return an empty message. Name the method and path, and for the two
/// route-level statuses say what it means and what to do, since "the daemon is
/// older than this MCP server" is the overwhelmingly likely cause and the agent
/// cannot guess it.
fn route_error(status: u16, method: &str, path: &str, body: &[u8]) -> (String, String) {
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if !text.is_empty() {
        return ("error".to_string(), text.to_string());
    }
    let version = env!("CARGO_PKG_VERSION");
    match status {
        404 => (
            "route_not_found".to_string(),
            format!(
                "the running Cosmonic Desktop daemon has no route for {method} {path}. \
                 This MCP server is version {version}; the daemon answering it is older \
                 and does not implement this endpoint. Update Cosmonic Desktop, then retry."
            ),
        ),
        405 => (
            "method_not_allowed".to_string(),
            format!(
                "the running Cosmonic Desktop daemon does not accept {method} on {path}. \
                 This MCP server is version {version}; either the daemon is older than it, \
                 or this is a bug in the MCP server. Update Cosmonic Desktop, then retry."
            ),
        ),
        _ => (
            "error".to_string(),
            format!("the daemon returned HTTP {status} for {method} {path} with no detail"),
        ),
    }
}

/// Connect to the daemon's control endpoint. The returned stream type differs
/// per platform (unix socket vs named pipe) but both are `AsyncRead +
/// AsyncWrite`, which is all the hyper handshake above needs.
#[cfg(unix)]
async fn connect(path: &std::path::Path) -> std::io::Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(path).await
}

/// Named-pipe connect (the `socket` is a `\\.\pipe\...` name on Windows).
///
/// `ERROR_PIPE_BUSY` is retried with a short bounded backoff: it means the
/// daemon is alive but every listening instance is momentarily mid-handshake
/// (the daemon keeps a pool precisely to make this rare) — the WaitNamedPipe
/// pattern, without the blocking win32 call. Any other error is immediately
/// `Unreachable` (daemon not running).
#[cfg(windows)]
async fn connect(
    path: &std::path::Path,
) -> std::io::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match ClientOptions::new().open(path.as_os_str()) {
            Ok(client) => return Ok(client),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY)
                    && std::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule the Connectors Directory enforces and the reason this function
    /// exists: an error a model reads must say WHAT went wrong. A message that
    /// is empty — or that is only a status code — is an automatic review fail,
    /// and it is what an empty-bodied axum 404/405 produced.
    #[test]
    fn an_empty_body_never_yields_an_empty_message() {
        for status in [400, 403, 404, 405, 409, 500, 502] {
            let (code, message) = route_error(status, "GET", "/v1/projects/x", b"");
            assert!(!code.is_empty(), "{status}: empty code");
            assert!(
                message.len() > 30,
                "{status}: message is not actionable: {message:?}"
            );
            assert!(
                !message.starts_with(' '),
                "{status}: message starts with a space: {message:?}"
            );
            assert!(
                message.contains("/v1/projects/x"),
                "{status}: message does not name the path: {message}"
            );
        }
    }

    #[test]
    fn route_level_statuses_name_the_version_skew() {
        // A new bundle spawned against an older running daemon is the most
        // likely cause, and the model cannot guess it.
        let (code, message) = route_error(404, "GET", "/v1/projects/x", b"");
        assert_eq!(code, "route_not_found");
        assert!(message.contains("GET"), "{message}");
        assert!(message.contains("Update Cosmonic Desktop"), "{message}");
        assert!(message.contains(env!("CARGO_PKG_VERSION")), "{message}");

        let (code, message) = route_error(405, "GET", "/v1/projects/x", b"");
        assert_eq!(code, "method_not_allowed");
        assert!(message.contains("does not accept GET"), "{message}");
        assert!(message.contains("Update Cosmonic Desktop"), "{message}");
    }

    #[test]
    fn a_json_error_keeps_whichever_of_code_and_message_it_carries() {
        // Only a body carrying NEITHER should be synthesized over.
        let parse = |body: &[u8]| {
            serde_json::from_slice::<Value>(body).ok().and_then(|v| {
                let code = v.get("code").and_then(|c| c.as_str());
                let message = v.get("message").and_then(|m| m.as_str());
                match (code, message) {
                    (None, None) => None,
                    (c, m) => Some((
                        c.unwrap_or("error").to_string(),
                        m.unwrap_or_default().to_string(),
                    )),
                }
            })
        };
        assert_eq!(
            parse(br#"{"message":"no code here"}"#),
            Some(("error".into(), "no code here".into()))
        );
        assert_eq!(
            parse(br#"{"code":"nope"}"#),
            Some(("nope".into(), String::new()))
        );
        assert_eq!(parse(br#"{"unrelated":1}"#), None);
    }

    #[test]
    fn a_non_json_body_is_reported_verbatim_rather_than_invented() {
        let (code, message) = route_error(500, "POST", "/v1/workloads", b"upstream exploded");
        assert_eq!(code, "error");
        assert_eq!(message, "upstream exploded");
    }

    #[test]
    fn a_whitespace_only_body_is_treated_as_empty() {
        // `"   "` is not detail; taking it verbatim reproduces the original bug
        // one layer down.
        let (code, message) = route_error(404, "GET", "/v1/thing", b"   \n\t ");
        assert_eq!(code, "route_not_found");
        assert!(message.contains("/v1/thing"), "{message}");
    }

    #[test]
    fn no_synthesized_message_is_multi_spaced() {
        // A `\` line continuation that got flattened leaves runs of spaces in
        // the middle of a sentence. It shipped once; catch it here.
        for status in [404, 405, 500] {
            let (_, message) = route_error(status, "GET", "/v1/x", b"");
            assert!(
                !message.contains("  "),
                "{status}: collapsed continuation in {message:?}"
            );
        }
    }
}
