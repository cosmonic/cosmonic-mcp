//! The Builder's launch backend for the workspace MCP App.
//!
//! Desktop's Build page (`builder_form.jsx`) scaffolds a project and then runs
//! a coding agent on it. Inside Desktop that last step lives in Electron's main
//! process: `externalTerminal.js` (which terminal apps exist, the launch
//! script, the detached spawn), `agents.js` `presence()` (which agents are
//! launchable) and `terminal.js` (login-shell discovery, the env filter). An
//! MCP App is a sandboxed iframe with no process of its own, so this module is
//! that main-process half for the app: the same registries, the same launch
//! script shapes, the same verdicts, answering the same `window.cosmonic`
//! calls the form already makes (`terminal.external.list` / `.launch`,
//! `agents.presence`, `projects.freeName`).
//!
//! The form model, the agent/model/mode matrix and the argv a launch runs stay
//! in Desktop's shared JS (`agentLaunch.js`); this side only resolves and
//! spawns. Keep the registries below in step with:
//!
//! - `MAC_TERMINALS` / `LINUX_TERMINALS` / `WIN_TERMINALS`: `externalTerminal.js`
//! - `AGENTS` (bin, skill dir, detect dirs): `agentLaunch.js` `LAUNCH_AGENTS`
//!   and `agents.js` `AGENTS`
//! - `ENV_DENY`: `terminal.js` `EXTRA_ENV_DENY`
//!
//! A launch starts a process on the user's machine, so it is the one thing
//! here a model must never reach: the tool is app-only, the server refuses it
//! outright for a client that does not render MCP Apps (see
//! `CosmonicMcp::renders_apps`), and `argv[0]` must be one of the agents
//! below. Custom agents and the integrated terminal dock stay Desktop-only.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// One launchable agent: its command word, its cosmonic-sandbox package dir
/// (home-relative) and the home dirs whose presence means "installed".
struct Agent {
    id: &'static str,
    name: &'static str,
    bin: &'static str,
    skill: &'static str,
    detect: &'static [&'static str],
}

const AGENTS: [Agent; 10] = [
    Agent {
        id: "claude",
        name: "Claude Code",
        bin: "claude",
        skill: ".claude/skills/cosmonic-sandbox",
        detect: &[".claude"],
    },
    Agent {
        id: "codex",
        name: "OpenAI Codex",
        bin: "codex",
        skill: ".agents/skills/cosmonic-sandbox",
        detect: &[".codex"],
    },
    Agent {
        id: "gemini",
        name: "Gemini CLI",
        bin: "gemini",
        skill: ".gemini/skills/cosmonic-sandbox",
        detect: &[".gemini"],
    },
    Agent {
        id: "antigravity",
        name: "Google Antigravity",
        bin: "agy",
        skill: ".gemini/skills/cosmonic-sandbox",
        detect: &[".gemini/antigravity-cli"],
    },
    Agent {
        id: "hermes",
        name: "Hermes Agent",
        bin: "hermes",
        skill: ".hermes/skills/cosmonic-sandbox",
        detect: &[".hermes"],
    },
    Agent {
        id: "pi",
        name: "Pi",
        bin: "pi",
        skill: ".pi/agent/skills/cosmonic-sandbox",
        detect: &[".pi"],
    },
    Agent {
        id: "kiro",
        name: "Kiro CLI",
        bin: "kiro-cli",
        skill: ".kiro/skills/cosmonic-sandbox",
        detect: &[".kiro"],
    },
    Agent {
        id: "openclaw",
        name: "OpenClaw",
        bin: "openclaw",
        skill: ".openclaw/skills/cosmonic-sandbox",
        detect: &[".openclaw"],
    },
    Agent {
        id: "opencode",
        name: "OpenCode",
        bin: "opencode",
        skill: ".config/opencode/skills/cosmonic-sandbox",
        detect: &[".config/opencode", ".opencode"],
    },
    Agent {
        id: "openshell",
        name: "NVIDIA OpenShell",
        bin: "openshell",
        skill: ".openshell/cosmonic",
        detect: &[".config/openshell", ".openshell"],
    },
];

/// How a terminal app takes the launch script.
#[derive(Clone, Copy)]
enum Style {
    /// macOS `open -a <App> <script>`: the app opens the executable script.
    OpenFile,
    /// macOS `open -na <App> --args <args>`, the args built by `ArgShape`.
    OpenArgs(ArgShape),
    /// POSIX spawn of the emulator binary itself.
    Spawn(ArgShape),
    /// Windows Terminal / PowerShell, hosting the generated .ps1.
    Windows,
}

/// Where the script goes on the terminal's command line.
#[derive(Clone, Copy)]
enum ArgShape {
    /// `-e <script>`
    Exec,
    /// `<script>`
    Bare,
    /// `start -- <script>` (WezTerm)
    Start,
    /// `-- <script>` (GNOME Terminal)
    DashDash,
    /// `-x <script>` (Xfce Terminal)
    X,
}

impl ArgShape {
    fn args(self, script: &str) -> Vec<String> {
        let s = script.to_string();
        match self {
            ArgShape::Exec => vec!["-e".into(), s],
            ArgShape::Bare => vec![s],
            ArgShape::Start => vec!["start".into(), "--".into(), s],
            ArgShape::DashDash => vec!["--".into(), s],
            ArgShape::X => vec!["-x".into(), s],
        }
    }
}

struct Terminal {
    id: &'static str,
    name: &'static str,
    /// App bundles (macOS) or the binary name (Linux, Windows).
    at: &'static [&'static str],
    style: Style,
}

const MAC_TERMINALS: [Terminal; 7] = [
    Terminal {
        id: "terminal",
        name: "Terminal",
        at: &[
            "/System/Applications/Utilities/Terminal.app",
            "/Applications/Utilities/Terminal.app",
        ],
        style: Style::OpenFile,
    },
    Terminal {
        id: "iterm",
        name: "iTerm2",
        at: &["/Applications/iTerm.app", "~/Applications/iTerm.app"],
        style: Style::OpenFile,
    },
    Terminal {
        id: "ghostty",
        name: "Ghostty",
        at: &["/Applications/Ghostty.app", "~/Applications/Ghostty.app"],
        style: Style::OpenArgs(ArgShape::Exec),
    },
    Terminal {
        id: "warp",
        name: "Warp",
        at: &["/Applications/Warp.app", "~/Applications/Warp.app"],
        style: Style::OpenFile,
    },
    Terminal {
        id: "wezterm",
        name: "WezTerm",
        at: &["/Applications/WezTerm.app", "~/Applications/WezTerm.app"],
        style: Style::OpenArgs(ArgShape::Start),
    },
    Terminal {
        id: "alacritty",
        name: "Alacritty",
        at: &[
            "/Applications/Alacritty.app",
            "~/Applications/Alacritty.app",
        ],
        style: Style::OpenArgs(ArgShape::Exec),
    },
    Terminal {
        id: "kitty",
        name: "kitty",
        at: &["/Applications/kitty.app", "~/Applications/kitty.app"],
        style: Style::OpenArgs(ArgShape::Bare),
    },
];

const LINUX_TERMINALS: [Terminal; 11] = [
    Terminal {
        id: "ghostty",
        name: "Ghostty",
        at: &["ghostty"],
        style: Style::Spawn(ArgShape::Exec),
    },
    Terminal {
        id: "kitty",
        name: "kitty",
        at: &["kitty"],
        style: Style::Spawn(ArgShape::Bare),
    },
    Terminal {
        id: "alacritty",
        name: "Alacritty",
        at: &["alacritty"],
        style: Style::Spawn(ArgShape::Exec),
    },
    Terminal {
        id: "wezterm",
        name: "WezTerm",
        at: &["wezterm"],
        style: Style::Spawn(ArgShape::Start),
    },
    Terminal {
        id: "gnome-terminal",
        name: "GNOME Terminal",
        at: &["gnome-terminal"],
        style: Style::Spawn(ArgShape::DashDash),
    },
    Terminal {
        id: "konsole",
        name: "Konsole",
        at: &["konsole"],
        style: Style::Spawn(ArgShape::Exec),
    },
    Terminal {
        id: "xfce4-terminal",
        name: "Xfce Terminal",
        at: &["xfce4-terminal"],
        style: Style::Spawn(ArgShape::X),
    },
    Terminal {
        id: "tilix",
        name: "Tilix",
        at: &["tilix"],
        style: Style::Spawn(ArgShape::Exec),
    },
    Terminal {
        id: "foot",
        name: "foot",
        at: &["foot"],
        style: Style::Spawn(ArgShape::Bare),
    },
    Terminal {
        id: "x-terminal-emulator",
        name: "System terminal",
        at: &["x-terminal-emulator"],
        style: Style::Spawn(ArgShape::Exec),
    },
    Terminal {
        id: "xterm",
        name: "xterm",
        at: &["xterm"],
        style: Style::Spawn(ArgShape::Exec),
    },
];

const WIN_TERMINALS: [Terminal; 2] = [
    Terminal {
        id: "wt",
        name: "Windows Terminal",
        at: &["wt.exe"],
        style: Style::Windows,
    },
    Terminal {
        id: "powershell",
        name: "PowerShell",
        at: &["powershell.exe"],
        style: Style::Windows,
    },
];

/// Launch-time env keys that never come from the app (`terminal.js`).
const ENV_DENY: [&str; 11] = [
    "PATH",
    "HOME",
    "SHELL",
    "USER",
    "LOGNAME",
    "USERPROFILE",
    "COMSPEC",
    "SYSTEMROOT",
    "TMPDIR",
    "TEMP",
    "TMP",
];

const BIN_MARKER: &str = "__COSMONIC_BIN__";

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Expand a leading `~` / `$HOME` (`terminal_discovery.js` `expandHome`).
fn expand_home(s: &str, home: &Path) -> String {
    let h = home.to_string_lossy();
    for prefix in ["${HOME}", "$HOME", "~"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if rest.is_empty() || rest.starts_with('/') || (prefix == "~" && rest.starts_with('\\'))
            {
                return format!("{h}{rest}");
            }
        }
    }
    s.to_string()
}

/// Single-quote for POSIX shells.
fn shq(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Single-quote for PowerShell.
fn psq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
        ["exe", "com", "cmd", "bat"]
            .iter()
            .map(|e| format!("{name}.{e}"))
            .collect()
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

fn is_csh(shell: &str) -> bool {
    matches!(
        Path::new(shell).file_name().and_then(|n| n.to_str()),
        Some("csh" | "tcsh")
    )
}

/// `$SHELL` when it is a real login shell (listed in /etc/shells where that
/// is readable), else the platform default (`terminal.js` `resolveShell`).
fn resolve_shell() -> String {
    if cfg!(windows) {
        return find_on_path("pwsh.exe")
            .or_else(|| find_on_path("powershell.exe"))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into()));
    }
    let fallback = if cfg!(target_os = "macos") {
        "/bin/zsh"
    } else {
        "/bin/bash"
    };
    let Ok(sh) = std::env::var("SHELL") else {
        return fallback.into();
    };
    if !Path::new(&sh).is_absolute() || !Path::new(&sh).exists() {
        return fallback.into();
    }
    if let Ok(list) = std::fs::read_to_string("/etc/shells") {
        let shells: Vec<&str> = list
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        if !shells.is_empty() && !shells.contains(&sh.as_str()) {
            return fallback.into();
        }
    }
    sh
}

// ---- Terminal detection ------------------------------------------------------

fn registry() -> &'static [Terminal] {
    if cfg!(target_os = "macos") {
        &MAC_TERMINALS
    } else if cfg!(windows) {
        &WIN_TERMINALS
    } else {
        &LINUX_TERMINALS
    }
}

/// A detected terminal and how to reach it (app bundle or binary path).
struct Found {
    term: &'static Terminal,
    at: PathBuf,
}

fn detect_terminals() -> Vec<Found> {
    let home = home();
    registry()
        .iter()
        .filter_map(|t| {
            let at = if cfg!(target_os = "macos") {
                t.at.iter()
                    .map(|p| PathBuf::from(expand_home(p, &home)))
                    .find(|p| p.exists())
            } else {
                // PowerShell ships with Windows; keep it even off PATH.
                find_on_path(t.at[0])
                    .or_else(|| (t.id == "powershell").then(|| PathBuf::from("powershell.exe")))
            };
            at.map(|at| Found { term: t, at })
        })
        .collect()
}

/// Desktop's persisted terminal choice (settings.json `terminalPrefs`), read
/// only: the app keeps the choice it makes in the iframe, so two processes
/// never write Desktop's settings file.
fn desktop_settings() -> Value {
    crate::ui_content::user_data()
        .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

pub fn terminals() -> Value {
    let found = detect_terminals();
    let external = desktop_settings()["terminalPrefs"]["external"]
        .as_str()
        .map(str::to_string);
    json!({
        "terminals": found.iter().map(|f| json!({"id": f.term.id, "name": f.term.name})).collect::<Vec<_>>(),
        // There is no integrated dock inside a chat host: launches always go
        // to a terminal app.
        "prefs": {"integrated": false, "external": external},
    })
}

// ---- Agent discovery -----------------------------------------------------------

#[derive(Clone, Default)]
struct Discovery {
    shell: String,
    /// bin name -> absolute path on disk.
    bins: BTreeMap<String, String>,
    /// bin names the login shell resolves only as an alias or function.
    via_shell: BTreeMap<String, bool>,
}

/// The last successful discovery: a launch resolves `argv[0]` from it, and a
/// failed re-probe keeps it rather than flipping agents to "not found".
static LAST: Mutex<Option<Discovery>> = Mutex::new(None);

fn probe_script(bins: &[&str], csh: bool) -> String {
    bins.iter()
        .map(|b| {
            let q = shq(b);
            if csh {
                format!("echo {}; which {q}; alias {q}", shq(&format!("{BIN_MARKER}{b}")))
            } else {
                format!(
                    "echo {}; command -v -- {q} 2>/dev/null; whence -p -- {q} 2>/dev/null; type -P -- {q} 2>/dev/null; alias -- {q} 2>/dev/null",
                    shq(&format!("{BIN_MARKER}{b}"))
                )
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The first absolute path in a probe line: a bare path, or the target of an
/// alias definition (`alias claude='/abs/claude'`, `claude: aliased to /abs`).
fn path_in(line: &str) -> Option<String> {
    let line = line.trim();
    if line.starts_with('/') && !line.contains(' ') {
        return Some(line.to_string());
    }
    let start = line.find(['/'])?;
    let before = &line[..start];
    if !(before.ends_with('\'')
        || before.ends_with('"')
        || before.ends_with(' ')
        || before.ends_with('=')
        || before.ends_with('`')
        || before.ends_with('('))
    {
        return None;
    }
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '`' | ')'))
        .unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Parse probe output into per-bin results.
fn parse_probe(out: &str, bins: &[&str], d: &mut Discovery) {
    let mut current: Option<&str> = None;
    for line in out.lines() {
        if let Some(name) = line.trim().strip_prefix(BIN_MARKER) {
            current = bins.iter().copied().find(|b| *b == name);
            continue;
        }
        let Some(bin) = current else { continue };
        if d.bins.contains_key(bin) {
            continue;
        }
        let l = line.trim();
        if l.is_empty() || l.contains("not found") || l.contains("Command not found") {
            continue;
        }
        match path_in(l) {
            Some(p) if Path::new(&p).is_file() => {
                d.bins.insert(bin.to_string(), p);
                d.via_shell.remove(bin);
            }
            _ => {
                d.via_shell.insert(bin.to_string(), true);
            }
        }
    }
}

async fn run_probe(shell: &str, args: &[&str]) -> Option<String> {
    let child = tokio::process::Command::new(shell)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let out = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.contains(BIN_MARKER).then_some(text)
}

/// Resolve every agent's binary the way the user's own terminal would: a
/// login + interactive shell (rc-file PATH edits and aliases apply), then a
/// plain login shell, then an interactive non-login pass for `.bashrc`-only
/// setups (`terminal.js` `discoverPosix`). Windows scans PATH.
async fn discover() -> Discovery {
    let shell = resolve_shell();
    let bins: Vec<&str> = AGENTS.iter().map(|a| a.bin).collect();
    let mut d = Discovery {
        shell: shell.clone(),
        ..Default::default()
    };
    if cfg!(windows) {
        for b in &bins {
            if let Some(p) = find_on_path(b) {
                d.bins
                    .insert(b.to_string(), p.to_string_lossy().into_owned());
            }
        }
    } else {
        let csh = is_csh(&shell);
        let script = probe_script(&bins, csh);
        let attempts: Vec<Vec<&str>> = if csh {
            vec![vec!["-c", &script]]
        } else {
            vec![vec!["-l", "-i", "-c", &script], vec!["-l", "-c", &script]]
        };
        let mut alive = false;
        for args in attempts {
            if let Some(out) = run_probe(&shell, &args).await {
                parse_probe(&out, &bins, &mut d);
                alive = true;
                break;
            }
        }
        if !csh
            && bins
                .iter()
                .any(|b| !d.bins.contains_key(*b) && !d.via_shell.contains_key(*b))
        {
            if let Some(out) = run_probe(&shell, &["-i", "-c", &script]).await {
                parse_probe(&out, &bins, &mut d);
                alive = true;
            }
        }
        if !alive {
            if let Some(last) = LAST.lock().unwrap().clone() {
                return last;
            }
        }
    }
    *LAST.lock().unwrap() = Some(d.clone());
    d
}

/// Desktop's binary pins (Settings → Agents), when they point at a file.
fn bin_overrides(home: &Path) -> BTreeMap<String, String> {
    let settings = desktop_settings();
    let mut out = BTreeMap::new();
    if let Some(map) = settings["agentConfig"]["binOverrides"].as_object() {
        for (id, p) in map {
            if let Some(p) = p.as_str() {
                let p = expand_home(p.trim(), home);
                if Path::new(&p).is_absolute() && Path::new(&p).is_file() {
                    out.insert(id.clone(), p);
                }
            }
        }
    }
    out
}

fn skill_installed(dir: &Path) -> bool {
    dir.join(".cosmonic-skill-version").exists() || dir.join("SKILL.md").exists()
}

/// `agents.js` `presence()`, for the agents this backend can launch: one row
/// per agent with Desktop's reason codes (`skill-missing`,
/// `binary-not-found`), so the form's verdicts and fix-it sentences read the
/// same as in Desktop.
pub async fn presence() -> Value {
    let home = home();
    let d = discover().await;
    let overrides = bin_overrides(&home);
    let mut rows = Vec::new();
    let mut skills = Vec::new();
    for a in &AGENTS {
        let skill_dir = home.join(a.skill);
        let installed = skill_installed(&skill_dir);
        let detected_dir = a.detect.iter().any(|p| home.join(p).exists());
        let overridden = overrides.get(a.id).cloned();
        let bin_path = overridden.clone().or_else(|| d.bins.get(a.bin).cloned());
        let via_shell = bin_path.is_none() && d.via_shell.contains_key(a.bin);
        let mut reasons = Vec::new();
        if !installed {
            reasons.push("skill-missing");
        }
        if bin_path.is_none() && !via_shell {
            reasons.push("binary-not-found");
        }
        rows.push(json!({
            "id": a.id, "name": a.name, "bin": a.bin, "custom": false,
            "detected": detected_dir || installed || bin_path.is_some() || via_shell,
            "skillInstalled": installed, "binPath": bin_path, "viaShell": via_shell, "foundInWsl": false,
            "overridden": overridden.is_some(),
            "mcpRegistration": null, "warnings": [],
            "launchable": reasons.is_empty(), "reasons": reasons,
        }));
        skills.push(json!({
            "id": a.id, "name": a.name, "kind": if a.id == "openshell" {"policy"} else {"skill"},
            "mcpOnly": false, "dest": skill_dir, "detected": detected_dir || installed,
            "bundled": true, "installed": installed, "installedVersion": null, "current": installed,
        }));
    }
    json!({
        "probe": {"shell": d.shell, "loginPath": null, "claudeEnv": null, "codexEnv": null},
        "config": {"customAgents": [], "models": [], "binOverrides": overrides},
        "skill": {"available": true, "bundledVersion": null, "agents": skills},
        "agents": rows,
    })
}

// ---- Project names ---------------------------------------------------------------

fn dns_label(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
}

/// `name`, or its first free numbered sibling in `dir` (`hello-api-2`): the
/// daemon refuses to scaffold into a non-empty directory (`index.js`
/// `projects:free-name`, `projectName.js`).
pub fn free_name(dir: &str, name: &str) -> Value {
    let dir = PathBuf::from(expand_home(dir.trim(), &home()));
    if !dns_label(name) || !dir.is_absolute() {
        return json!({"name": name, "taken": false});
    }
    let taken = |n: &str| {
        let p = dir.join(n);
        match std::fs::metadata(&p) {
            Err(_) => false,
            Ok(m) if !m.is_dir() => true,
            Ok(_) => std::fs::read_dir(&p)
                .map(|mut r| r.next().is_some())
                .unwrap_or(true),
        }
    };
    let free = std::iter::once(name.to_string())
        .chain((2..=99).map(|n| {
            let suffix = format!("-{n}");
            let head: String = name.chars().take(63 - suffix.len()).collect();
            format!("{}{suffix}", head.trim_end_matches('-'))
        }))
        .find(|n| !taken(n))
        .unwrap_or_else(|| name.to_string());
    json!({"taken": free != name, "name": free})
}

// ---- Launch ----------------------------------------------------------------------

/// The launch-time env the app may set: shell-safe names, never the keys that
/// would redirect the process (`terminal.js` `sanitizeExtraEnv`).
fn sanitize_env(env: Option<&Map<String, Value>>, home: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in env.into_iter().flatten() {
        let ok_name = k.len() <= 128
            && k.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        let denied = ENV_DENY.iter().any(|d| {
            if cfg!(windows) {
                d.eq_ignore_ascii_case(k)
            } else {
                d == k
            }
        });
        if let (true, false, Some(v)) = (ok_name, denied, v.as_str()) {
            if v.len() <= 4096 && !v.contains('\0') {
                out.insert(k.clone(), expand_home(v, home));
            }
        }
    }
    out
}

/// Bare command words stay unquoted so the login shell can alias-expand them.
fn bare_word(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+~/-".contains(c))
}

/// The POSIX launch script: title, cd, then the user's login shell running the
/// agent (`externalTerminal.js` `buildPosixLaunchScript`).
fn posix_script(
    shell: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
    argv: &[String],
    title: &str,
) -> String {
    let csh = is_csh(shell);
    let mut lines = vec![
        "#!/bin/sh".to_string(),
        "# Generated by Cosmonic Desktop. Safe to delete.".to_string(),
    ];
    if !title.is_empty() {
        lines.push(format!("printf '\\033]2;%s\\007' {}", shq(title)));
    }
    lines.push(format!("cd {} || exit 1", shq(cwd)));
    let word = if bare_word(&argv[0]) {
        argv[0].clone()
    } else {
        shq(&argv[0])
    };
    let mut inner: Vec<String> = env
        .iter()
        .map(|(k, v)| {
            if csh {
                format!("setenv {k} {}", shq(v))
            } else {
                format!("export {k}={}", shq(v))
            }
        })
        .collect();
    inner.push(
        std::iter::once(word)
            .chain(argv[1..].iter().map(|a| shq(a)))
            .collect::<Vec<_>>()
            .join(" "),
    );
    let inner = inner.join("; ");
    lines.push(if csh {
        format!("exec {} -c {}", shq(shell), shq(&inner))
    } else {
        format!("exec {} -l -i -c {}", shq(shell), shq(&inner))
    });
    lines.join("\n") + "\n"
}

/// The PowerShell launch script; the BOM keeps PowerShell 5.1 reading UTF-8.
/// Native-command args pre-escape `"` (`externalTerminal.js` `psNativeArg`).
fn powershell_script(
    cwd: &str,
    env: &BTreeMap<String, String>,
    argv: &[String],
    title: &str,
) -> String {
    let mut lines = vec!["# Generated by Cosmonic Desktop. Safe to delete.".to_string()];
    if !title.is_empty() {
        lines.push(format!("$Host.UI.RawUI.WindowTitle = {}", psq(title)));
    }
    lines.push(format!("Set-Location -LiteralPath {}", psq(cwd)));
    for (k, v) in env {
        lines.push(format!("$env:{k} = {}", psq(v)));
    }
    let args: Vec<String> = argv[1..]
        .iter()
        .map(|a| psq(&a.replace('"', "\\\"")))
        .collect();
    lines.push(
        format!("& {} {}", psq(&argv[0]), args.join(" "))
            .trim_end()
            .to_string(),
    );
    format!("\u{FEFF}{}\r\n", lines.join("\r\n"))
}

/// Scripts live in this user's own data dir (0700), never the shared temp dir
/// where a co-user could swap a predictable path between write and exec.
/// Scripts older than an hour are swept on each launch.
fn write_script(content: &str, title: &str, ext: &str) -> std::io::Result<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| std::io::Error::other("no data directory"))?
        .join("cosmonic-mcp")
        .join("launch-scripts");
    std::fs::create_dir_all(&dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok());
            if old.is_some_and(|age| age > Duration::from_secs(3600)) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let slug: String = title
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = if slug.is_empty() {
        "shell".into()
    } else {
        slug.chars().take(40).collect()
    };
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = dir.join(format!("{slug}-{nonce:x}{ext}"));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o700);
    }
    use std::io::Write;
    opts.open(&path)?.write_all(content.as_bytes())?;
    Ok(path)
}

/// Spawn detached and confirm it started. `wait` also waits for the exit code
/// (macOS `open` exits at once, and its status is the only failure signal).
async fn spawn_detached(
    file: &Path,
    args: &[String],
    cwd: Option<&Path>,
    wait: bool,
) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new(file);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if !wait {
        return Ok(());
    }
    match tokio::time::timeout(Duration::from_secs(10), child.wait()).await {
        Ok(Ok(status)) if status.success() => Ok(()),
        Ok(Ok(status)) => Err(format!("launcher exited with {status}")),
        Ok(Err(e)) => Err(e.to_string()),
        // A lingering launcher most likely worked; do not block the form.
        Err(_) => Ok(()),
    }
}

fn str_list(v: Option<&Value>) -> Option<Vec<String>> {
    v?.as_array()?
        .iter()
        .map(|s| s.as_str().map(str::to_string))
        .collect()
}

/// Run `argv` in the chosen terminal app, cd'd into `cwd`, as an independent
/// process (`externalTerminal.js` `launchExternal`). `argv[0]` must name one
/// of `AGENTS`; it resolves through Desktop's pin, then discovery, else stays
/// bare for the login shell to resolve.
pub async fn launch(args: &Map<String, Value>) -> Result<Value, String> {
    let home = home();
    let argv = str_list(args.get("argv"))
        .filter(|a| !a.is_empty())
        .ok_or("`argv` is required")?;
    if argv.len() > 64
        || argv.iter().any(|a| a.contains('\0'))
        || argv.iter().map(String::len).sum::<usize>() > 256 * 1024
    {
        return Err("`argv` is too large".into());
    }
    let agent = AGENTS
        .iter()
        .find(|a| a.bin == argv[0])
        .ok_or_else(|| format!("`{}` is not an agent the Builder launches", argv[0]))?;
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(|c| expand_home(c, &home))
        .ok_or("`cwd` is required")?;
    if !Path::new(&cwd).is_absolute() || !Path::new(&cwd).is_dir() {
        return Err(format!("{cwd} is not a directory"));
    }
    let title: String = args
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    let env = sanitize_env(args.get("env").and_then(Value::as_object), &home);
    let found = detect_terminals();
    let wanted = args.get("terminal").and_then(Value::as_str);
    let f = found
        .iter()
        .find(|f| Some(f.term.id) == wanted)
        .or_else(|| found.first())
        .ok_or("No terminal application was found on this system.")?;

    let mut argv = argv;
    let pinned = bin_overrides(&home).remove(agent.id);
    let discovered = LAST
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|d| d.bins.get(agent.bin).cloned());
    if let Some(path) = pinned.or(discovered) {
        argv[0] = path;
    }

    let name = f.term.name;
    let fail = |e: String| format!("Couldn't open {name}: {e}");
    match f.term.style {
        Style::Windows => {
            let script = write_script(
                &powershell_script(&cwd, &env, &argv, &title),
                &title,
                ".ps1",
            )
            .map_err(|e| fail(e.to_string()))?;
            let script = script.to_string_lossy().into_owned();
            let ps: Vec<String> = ["-NoExit", "-ExecutionPolicy", "Bypass", "-File", &script]
                .iter()
                .map(|s| s.to_string())
                .collect();
            if f.term.id == "wt" {
                let mut a = vec!["-d".to_string(), cwd.clone(), "powershell.exe".to_string()];
                a.extend(ps);
                spawn_detached(&f.at, &a, None, false).await.map_err(fail)?;
            } else {
                // `cmd /c start` gives PowerShell a visible console window.
                let mut a = vec![
                    "/c".to_string(),
                    "start".to_string(),
                    String::new(),
                    "powershell.exe".to_string(),
                ];
                a.extend(ps);
                spawn_detached(Path::new("cmd.exe"), &a, Some(Path::new(&cwd)), false)
                    .await
                    .map_err(fail)?;
            }
        }
        style => {
            let shell = resolve_shell();
            let ext = if cfg!(target_os = "macos") {
                ".command"
            } else {
                ".sh"
            };
            let script = write_script(
                &posix_script(&shell, &cwd, &env, &argv, &title),
                &title,
                ext,
            )
            .map_err(|e| fail(e.to_string()))?;
            let script = script.to_string_lossy().into_owned();
            let app = f.at.to_string_lossy().into_owned();
            match style {
                Style::OpenFile => {
                    spawn_detached(
                        Path::new("/usr/bin/open"),
                        &["-a".into(), app, script],
                        None,
                        true,
                    )
                    .await
                }
                Style::OpenArgs(shape) => {
                    let mut a = vec!["-na".to_string(), app, "--args".to_string()];
                    a.extend(shape.args(&script));
                    spawn_detached(Path::new("/usr/bin/open"), &a, None, true).await
                }
                Style::Spawn(shape) => {
                    spawn_detached(&f.at, &shape.args(&script), None, false).await
                }
                Style::Windows => unreachable!(),
            }
            .map_err(fail)?;
        }
    }
    Ok(json!({"ok": true, "terminal": f.term.id}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_script_execs_the_login_shell_with_quoted_args() {
        let env = BTreeMap::from([("ANTHROPIC_MODEL".to_string(), "x'y".to_string())]);
        let argv = vec![
            "claude".to_string(),
            "--permission-mode".to_string(),
            "auto".to_string(),
            "it's a prompt".to_string(),
        ];
        let s = posix_script("/bin/zsh", "/Users/me/p q", &env, &argv, "Claude Code · p");
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("cd '/Users/me/p q' || exit 1"));
        assert!(s.contains("exec '/bin/zsh' -l -i -c "));
        // The agent word stays bare so aliases apply; the prompt is quoted.
        assert!(s.contains("claude "), "{s}");
        let inner = format!(
            "export ANTHROPIC_MODEL={}; claude '--permission-mode' 'auto' {}",
            shq("x'y"),
            shq("it's a prompt")
        );
        assert!(s.contains(&shq(&inner)), "{s}");
        let csh = posix_script("/bin/tcsh", "/tmp", &env, &argv, "");
        assert!(
            csh.contains("setenv ANTHROPIC_MODEL") && csh.contains("exec '/bin/tcsh' -c "),
            "{csh}"
        );
    }

    #[test]
    fn powershell_script_has_a_bom_and_escapes_quotes() {
        let s = powershell_script(
            r"C:\p",
            &BTreeMap::new(),
            &["claude".into(), r#"say "hi""#.into()],
            "t",
        );
        assert!(s.starts_with('\u{FEFF}'));
        assert!(s.contains(r#"& 'claude' 'say \"hi\"'"#), "{s}");
    }

    #[test]
    fn env_filter_drops_redirecting_and_malformed_keys() {
        let env = json!({"PATH":"/evil","HOME":"/x","OPENSHELL_SANDBOX_POLICY":"~/.openshell/p.yaml","bad-key":"v","N":1});
        let out = sanitize_env(env.as_object(), Path::new("/Users/me"));
        assert_eq!(out.len(), 1);
        assert_eq!(
            out["OPENSHELL_SANDBOX_POLICY"],
            "/Users/me/.openshell/p.yaml"
        );
    }

    #[tokio::test]
    async fn launch_refuses_anything_but_a_known_agent() {
        let args = json!({"argv":["/bin/sh","-c","echo pwned"],"cwd":"/tmp"});
        let err = launch(args.as_object().unwrap()).await.unwrap_err();
        assert!(err.contains("not an agent"), "{err}");
        let args = json!({"argv":["claude"],"cwd":"relative/dir"});
        assert!(launch(args.as_object().unwrap()).await.is_err());
    }

    // The login-shell probe is POSIX-only (Windows looks agents up on PATH),
    // so its parser only knows POSIX paths.
    #[cfg(unix)]
    #[test]
    fn probe_parsing_takes_paths_and_aliases() {
        let real = std::env::current_exe().unwrap();
        let out = format!(
            "{BIN_MARKER}claude\n{}\n{BIN_MARKER}codex\nalias codex='npx codex'\n{BIN_MARKER}gemini\n",
            real.display()
        );
        let mut d = Discovery::default();
        parse_probe(&out, &["claude", "codex", "gemini"], &mut d);
        assert_eq!(d.bins.get("claude").map(PathBuf::from), Some(real));
        assert!(d.via_shell.contains_key("codex"));
        assert!(!d.bins.contains_key("gemini") && !d.via_shell.contains_key("gemini"));
    }

    #[test]
    fn alias_targets_are_extracted() {
        assert_eq!(
            path_in("alias claude='/opt/c/claude --x'").as_deref(),
            Some("/opt/c/claude")
        );
        assert_eq!(
            path_in("claude: aliased to /opt/c/claude").as_deref(),
            Some("/opt/c/claude")
        );
        assert_eq!(
            path_in("/usr/local/bin/codex").as_deref(),
            Some("/usr/local/bin/codex")
        );
        assert_eq!(path_in("codex not found"), None);
    }

    #[test]
    fn free_names_number_upward() {
        let dir = std::env::temp_dir().join(format!("cosmonic-free-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("api")).unwrap();
        std::fs::write(dir.join("api").join("f"), "x").unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        let d = dir.to_string_lossy();
        assert_eq!(free_name(&d, "api"), json!({"name":"api-2","taken":true}));
        assert_eq!(
            free_name(&d, "empty"),
            json!({"name":"empty","taken":false})
        );
        assert_eq!(
            free_name(&d, "Not_A_Label"),
            json!({"name":"Not_A_Label","taken":false})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
