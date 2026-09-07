//! Shared telemetry vocabulary.
//!
//! Lives here, in the crate every process already depends on, because two
//! processes emit the same events and they must agree on the values: the
//! daemon (`cosmonicd`'s `usage/` module — the one door to Amplitude) and the
//! MCP server (`cosmonic-mcp`), which hands its events to the daemon over the
//! socket. A second copy of `normalize_mcp_client` in the MCP crate would
//! silently drift into reporting a client the catalog's allowlist rejects,
//! dropping the event with nothing to show for it.
//!
//! Everything here is deliberately LOSSY. `clientInfo.name` is chosen by the
//! client and routinely carries a path or a hostname; a full version string is
//! a fingerprinting surface; a raw duration or count is a behavioural one. The
//! wire only ever sees a fixed enum, a major version, or a bucket label.

/// MCP client identities recognized from the `initialize` handshake's
/// `clientInfo.name`. Anything else maps to `other` — the raw string never
/// travels.
pub const MCP_CLIENTS: &[&str] = &[
    "claude_code",
    "claude_desktop",
    "cursor",
    "windsurf",
    "hermes",
    "openshell",
    "openclaw",
    "opencode",
    "codex",
    "vscode",
    "other",
];

/// Map an MCP `clientInfo.name` to the fixed [`MCP_CLIENTS`] enum.
///
/// Matching is case- and punctuation-insensitive, then a short alias table.
/// An unrecognized name is `other`, never the input.
pub fn normalize_mcp_client(raw: &str) -> &'static str {
    let norm: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let norm = norm.trim_matches('_').to_string();
    // Exact hits first, then a couple of well-known aliases.
    if let Some(c) = MCP_CLIENTS.iter().find(|c| **c == norm) {
        return c;
    }
    match norm.as_str() {
        "claude_ai" | "claude" | "claudecode" => "claude_code",
        "claude_desktop_app" | "claude_for_desktop" => "claude_desktop",
        "cursor_vscode" | "cursor_ide" => "cursor",
        "visual_studio_code" | "vscode_mcp" | "code" => "vscode",
        "windsurf_ide" => "windsurf",
        _ => "other",
    }
}

/// The MAJOR component of a semver-ish version string, or `"unknown"`.
/// Deliberately lossy: a full version is a fingerprinting surface.
pub fn version_major(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .next()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
        .unwrap_or("unknown")
        .to_string()
}

/// Session length: `<1m` / `1-10m` / `10-60m` / `1-8h` / `8h+`.
pub fn session_secs(secs: u64) -> &'static str {
    match secs {
        0..=59 => "<1m",
        60..=599 => "1-10m",
        600..=3_599 => "10-60m",
        3_600..=28_799 => "1-8h",
        _ => "8h+",
    }
}

/// Small counts: `0` / `1-5` / `6-20` / `21+`.
pub fn count_results(n: u64) -> &'static str {
    match n {
        0 => "0",
        1..=5 => "1-5",
        6..=20 => "6-20",
        _ => "21+",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_client_never_leaks_its_raw_name() {
        // The whole point: clientInfo.name is attacker-controlled and often
        // carries a path or a hostname.
        for raw in [
            "/Users/someone/bin/weird-agent",
            "acme-internal-tool 1.2",
            "host.corp.example",
            "",
        ] {
            let got = normalize_mcp_client(raw);
            assert_eq!(got, "other", "{raw:?} -> {got}");
        }
    }

    #[test]
    fn known_clients_and_aliases_map_to_the_enum() {
        assert_eq!(normalize_mcp_client("claude-code"), "claude_code");
        assert_eq!(normalize_mcp_client("Claude Code"), "claude_code");
        assert_eq!(normalize_mcp_client("claude"), "claude_code");
        assert_eq!(normalize_mcp_client("Claude Desktop"), "claude_desktop");
        assert_eq!(normalize_mcp_client("Visual Studio Code"), "vscode");
        assert_eq!(normalize_mcp_client("opencode"), "opencode");
    }

    #[test]
    fn every_result_is_in_the_allowlist() {
        // A value outside MCP_CLIENTS is dropped by the catalog, so the event
        // would vanish with nothing to show for it.
        for raw in ["claude-code", "nonsense", "", "cursor_ide", "windsurf-ide"] {
            assert!(
                MCP_CLIENTS.contains(&normalize_mcp_client(raw)),
                "{raw:?} produced a value the catalog would reject"
            );
        }
    }

    #[test]
    fn only_the_major_version_travels() {
        assert_eq!(version_major("1.2.3"), "1");
        assert_eq!(version_major("v2.0.0-beta.1"), "2");
        assert_eq!(version_major("2026.09.06"), "2026");
        assert_eq!(version_major("not-a-version"), "unknown");
        assert_eq!(version_major(""), "unknown");
    }

    #[test]
    fn buckets_cover_their_ranges_without_gaps() {
        assert_eq!(session_secs(0), "<1m");
        assert_eq!(session_secs(59), "<1m");
        assert_eq!(session_secs(60), "1-10m");
        assert_eq!(session_secs(3_600), "1-8h");
        assert_eq!(session_secs(u64::MAX), "8h+");
        assert_eq!(count_results(0), "0");
        assert_eq!(count_results(5), "1-5");
        assert_eq!(count_results(6), "6-20");
        assert_eq!(count_results(u64::MAX), "21+");
    }
}
