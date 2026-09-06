//! Where the daemon's control endpoint is.
//!
//! A third copy of this resolution — `cosmonic-host::paths` is the original,
//! `cosmonic-stdio::daemon` the second — and for the same reason both others
//! exist: a binary that must stay small enough to ship inside an `.mcpb`, and
//! open-sourceable on its own, cannot depend on `cosmonic-host` without
//! dragging the whole wash-runtime/wasmtime build in behind it.
//!
//! **Keep in step with `cosmonic-host::paths`.** The daemon decides where the
//! socket goes; this only has to find it. A divergence surfaces as
//! `daemon_unreachable` on every call, which reads as "Cosmonic Desktop is not
//! running" — so it is worth the duplication being loud rather than clever.
//!
//! Resolution order, matching the daemon:
//!   1. `COSMONIC_SOCKET` — an explicit endpoint, wins outright.
//!   2. `COSMONIC_STATE_DIR` (unix) — the socket lives beside the state dir.
//!   3. The platform default.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Resolve the daemon control endpoint exactly as the daemon does.
pub fn socket_path() -> Result<PathBuf> {
    if let Some(v) = std::env::var_os("COSMONIC_SOCKET") {
        if !v.is_empty() {
            return Ok(PathBuf::from(v));
        }
    }
    #[cfg(unix)]
    {
        if let Some(state) = std::env::var_os("COSMONIC_STATE_DIR") {
            if !state.is_empty() {
                return Ok(PathBuf::from(state).join("cosmonicd.sock"));
            }
        }
    }
    platform_socket_path()
}

/// Reject a unix socket path that can never connect.
///
/// `sockaddr_un.sun_path` is 104 bytes on macOS including the NUL. Without this
/// the failure surfaces as `daemon unreachable: path must be shorter than
/// SUN_LEN` on every call, which reads as "the daemon isn't running".
#[cfg(unix)]
pub fn check_socket_path(path: &std::path::Path) -> Result<()> {
    // 104 on macOS/BSD, 108 on Linux; use the smaller so a path that works here
    // works everywhere.
    const MAX: usize = 103;
    let len = path.as_os_str().as_encoded_bytes().len();
    anyhow::ensure!(
        len <= MAX,
        "the daemon socket path is {len} bytes, but this platform allows at most {MAX}: {}. \
         Point COSMONIC_STATE_DIR at a shorter directory.",
        path.display()
    );
    Ok(())
}

/// No length limit applies to a Windows named pipe name.
#[cfg(windows)]
pub fn check_socket_path(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn platform_socket_path() -> Result<PathBuf> {
    if let Some(rt) = std::env::var_os("XDG_RUNTIME_DIR") {
        if !rt.is_empty() {
            return Ok(PathBuf::from(rt).join("cosmonic").join("cosmonicd.sock"));
        }
    }
    let data = dirs::data_local_dir().context("could not resolve a data directory")?;
    Ok(data.join("cosmonic").join("cosmonicd.sock"))
}

#[cfg(all(unix, not(target_os = "linux")))]
fn platform_socket_path() -> Result<PathBuf> {
    // macOS: ~/Library/Application Support/Cosmonic/cosmonicd.sock
    let data = dirs::data_local_dir().context("could not resolve a data directory")?;
    Ok(data.join("Cosmonic").join("cosmonicd.sock"))
}

#[cfg(windows)]
fn platform_socket_path() -> Result<PathBuf> {
    // The endpoint is a named pipe whose name hashes the user scope and
    // COSMONIC_STATE_DIR. Keep in step with `cosmonic-host::paths`.
    Ok(PathBuf::from(windows_pipe_name()))
}

#[cfg(windows)]
fn windows_pipe_name() -> String {
    let scope = std::env::var("USERNAME").unwrap_or_default();
    let state = std::env::var("COSMONIC_STATE_DIR").unwrap_or_default();
    format!(
        r"\\.\pipe\cosmonicd-{:016x}",
        fnv1a64(&format!("{scope}|{state}"))
    )
}

#[cfg(windows)]
fn fnv1a64(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize the env-var mutation these tests do — `cargo test` runs them
    /// on threads that share one environment.
    static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn an_explicit_socket_wins_over_everything() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("COSMONIC_SOCKET", "/tmp/explicit.sock");
        std::env::set_var("COSMONIC_STATE_DIR", "/tmp/state");
        let got = socket_path().unwrap();
        std::env::remove_var("COSMONIC_SOCKET");
        std::env::remove_var("COSMONIC_STATE_DIR");
        assert_eq!(got, PathBuf::from("/tmp/explicit.sock"));
    }

    #[cfg(unix)]
    #[test]
    fn a_state_dir_puts_the_socket_beside_it() {
        let _guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("COSMONIC_SOCKET");
        std::env::set_var("COSMONIC_STATE_DIR", "/tmp/cosmo-state");
        let got = socket_path().unwrap();
        std::env::remove_var("COSMONIC_STATE_DIR");
        assert_eq!(got, PathBuf::from("/tmp/cosmo-state/cosmonicd.sock"));
    }

    #[cfg(unix)]
    #[test]
    fn an_over_long_socket_path_is_refused_with_the_fix_in_the_message() {
        let long = PathBuf::from(format!("/tmp/{}/cosmonicd.sock", "x".repeat(120)));
        let err = check_socket_path(&long).unwrap_err().to_string();
        assert!(err.contains("COSMONIC_STATE_DIR"), "{err}");
        assert!(check_socket_path(std::path::Path::new("/tmp/a.sock")).is_ok());
    }
}
