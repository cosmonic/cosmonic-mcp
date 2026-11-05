//! Launchpad and Academy content for the workspace UI.
//!
//! Cosmonic Desktop's Electron process downloads both hubs' content releases
//! and extracts them under its user-data directory
//! (`<userData>/content/<hub>/<sourceId>/`, src/main/content/store.ts). This
//! module reads those trees, read-only, for the embedded app, which runs
//! Desktop's own catalog parser and merger (`src/shared/content`) on them. It
//! keeps Desktop's rules: the same source list (`settings.json`
//! `contentSources` / `contentOverrides`), the same path jail
//! (`paths.js` safeSegments/jailResolve), the same MIME allowlist and caps.
//! Downloading updates and adding sources stay with the Desktop app.
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub const HUBS: [&str; 2] = ["launchpad", "academy"];
const MAX_INDEX_BYTES: u64 = 5 * 1024 * 1024;
const MAX_ASSET_BYTES: u64 = 5 * 1024 * 1024;
/// A batch answers at most this many bytes of file content, so one call can
/// never ask the host to ferry an unbounded payload.
const MAX_BATCH_BYTES: u64 = 16 * 1024 * 1024;

fn builtin_repo(hub: &str) -> &'static str {
    if hub == "academy" {
        "cosmonic-labs/academy"
    } else {
        "cosmonic-labs/launchpad"
    }
}

/// Electron's `app.getPath("userData")` for Cosmonic Desktop: the config dir
/// plus the product name (`cosmonic-desktop` on Linux packages).
pub fn user_data() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("COSMONIC_DESKTOP_USER_DATA") {
        return Some(PathBuf::from(dir));
    }
    let base = dirs::config_dir()?;
    ["Cosmonic Desktop", "cosmonic-desktop"]
        .iter()
        .map(|n| base.join(n))
        .find(|p| p.exists())
}

fn settings() -> Value {
    user_data()
        .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null)
}

/// `safeSegments` from Desktop's src/main/content/paths.ts.
fn safe_segments(path: &str) -> Option<Vec<&str>> {
    let segs: Vec<&str> = path.split('/').collect();
    if segs.is_empty() || segs.len() > 16 {
        return None;
    }
    segs.iter()
        .all(|s| {
            !s.is_empty() && s.len() <= 255 && !s.starts_with('.') && !s.contains(['\\', '\0'])
        })
        .then_some(segs)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 63
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

fn valid_repo(repo: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
            && !s.starts_with(['.', '-'])
    };
    repo.len() <= 200
        && matches!(repo.split_once('/'), Some((a, b)) if ok(a) && ok(b) && !b.contains('/'))
}

/// Desktop's `store.sources(hub)`: the builtin source first, then the
/// user-added ones from settings.json, deduplicated by id and repo.
fn sources(hub: &str) -> Vec<(String, String, bool)> {
    let mut out = vec![("cosmonic".to_string(), builtin_repo(hub).to_string(), true)];
    if let Some(list) = settings()
        .pointer(&format!("/contentSources/{hub}"))
        .and_then(Value::as_array)
    {
        for s in list {
            let (Some(id), Some(repo)) = (s["id"].as_str(), s["repo"].as_str()) else {
                continue;
            };
            if !valid_id(id)
                || !valid_repo(repo)
                || out
                    .iter()
                    .any(|(i, r, _)| i == id || r.eq_ignore_ascii_case(repo))
            {
                continue;
            }
            out.push((id.to_string(), repo.to_string(), false));
        }
    }
    out
}

fn source_root(hub: &str, source: &str) -> Option<PathBuf> {
    if !HUBS.contains(&hub) || !sources(hub).iter().any(|(id, _, _)| id == source) {
        return None;
    }
    Some(user_data()?.join("content").join(hub).join(source))
}

/// `jailResolve`: the path must stay under the root after symlinks resolve.
fn jail(root: &Path, rel: &str) -> Result<PathBuf, &'static str> {
    let segs = safe_segments(rel).ok_or("invalid path")?;
    let full = segs.iter().fold(root.to_path_buf(), |p, s| p.join(s));
    let real_root = root
        .canonicalize()
        .map_err(|_| "no content installed for this source")?;
    let real = full.canonicalize().map_err(|_| "not found")?;
    if !real.starts_with(&real_root) {
        return Err("path escapes the content directory");
    }
    Ok(real)
}

fn mime_for(path: &Path) -> Option<(&'static str, bool)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "svg" => ("image/svg+xml", false),
        "png" => ("image/png", false),
        "jpg" | "jpeg" => ("image/jpeg", false),
        "gif" => ("image/gif", false),
        "webp" => ("image/webp", false),
        "md" => ("text/markdown", true),
        "txt" => ("text/plain", true),
        "yaml" | "yml" => ("text/yaml", true),
        "json" => ("application/json", true),
        _ => return None,
    })
}

fn read_file(hub: &str, source: &str, rel: &str, budget: &mut u64) -> Value {
    let mut attempt = || -> Result<Value, String> {
        let root = source_root(hub, source).ok_or("unknown content source")?;
        let file = jail(&root, rel)?;
        if rel.split('/').count() == 1 && rel.eq_ignore_ascii_case("state.json") {
            return Err("not served".into());
        }
        let (mime, text) = mime_for(&file).ok_or("file type not served")?;
        let meta = std::fs::metadata(&file).map_err(|e| e.to_string())?;
        if !meta.is_file() {
            return Err("not a file".into());
        }
        if meta.len() > MAX_ASSET_BYTES {
            return Err("file exceeds the 5 MB cap".into());
        }
        if meta.len() > *budget {
            return Err("batch size cap reached; request it again".into());
        }
        *budget -= meta.len();
        let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
        Ok(if text {
            json!({"ok":true,"mime":mime,"text":String::from_utf8_lossy(&bytes)})
        } else {
            json!({"ok":true,"mime":mime,"base64":base64(&bytes)})
        })
    };
    attempt().unwrap_or_else(|e| json!({"ok":false,"error":e}))
}

/// Everything `content:get` needs to build a hub payload: per source, its
/// raw index.json and state.json. Parsing and merging happen in the app with
/// Desktop's own code.
pub fn hub(hub: &str) -> Result<Value, String> {
    if !HUBS.contains(&hub) {
        return Err(format!("unknown content hub {hub:?}"));
    }
    let Some(data) = user_data() else {
        return Ok(json!({"installed":false,"override":false,"sources":[]}));
    };
    let override_builtin =
        settings().pointer(&format!("/contentOverrides/{hub}")) == Some(&Value::Bool(true));
    let list: Vec<Value> = sources(hub)
        .into_iter()
        .map(|(id, repo, builtin)| {
            let dir = data.join("content").join(hub).join(&id);
            let index = std::fs::metadata(dir.join("index.json"))
                .ok()
                .filter(|m| m.len() <= MAX_INDEX_BYTES)
                .and_then(|_| std::fs::read_to_string(dir.join("index.json")).ok());
            let state = std::fs::read_to_string(dir.join("state.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .unwrap_or(json!({}));
            json!({"id":id,"repo":repo,"builtin":builtin,"index":index,"state":state})
        })
        .collect();
    Ok(json!({"installed":true,"override":override_builtin,"sources":list}))
}

/// Read up to 64 files `{hub, source, path}` from the content trees.
pub fn files(requests: &[Value]) -> Result<Value, String> {
    if requests.is_empty() || requests.len() > 64 {
        return Err("`files` takes 1 to 64 entries".into());
    }
    let mut budget = MAX_BATCH_BYTES;
    let results: Vec<Value> = requests
        .iter()
        .map(|r| {
            let field = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
            read_file(field("hub"), field("source"), field("path"), &mut budget)
        })
        .collect();
    Ok(json!({"ok":true,"results":results}))
}

pub fn call(args: &Map<String, Value>) -> Result<Value, String> {
    match args.get("action").and_then(Value::as_str) {
        Some("hub") => hub(args.get("hub").and_then(Value::as_str).unwrap_or("")),
        // The user's home: Desktop's project-folder default (`~/cosmonic`) is
        // resolved against it (data.js resolveProjectsDir).
        // Whether the Cosmonic Desktop app is installed, for the first-run
        // screen: "install it" and "start it" are different answers.
        Some("env") => Ok(
            json!({"ok":true,"home":dirs::home_dir().map(|h| h.display().to_string()),
            "desktop":{"installed":desktop_app().is_some(),"path":desktop_app().map(|p| p.display().to_string())}}),
        ),
        Some("files") => files(
            args.get("files")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        ),
        _ => Err("`action` must be hub, files or env".into()),
    }
}

/// Where the Cosmonic Desktop app is installed, if it is. Checks the
/// locations its installers use (electron-builder.yml: dmg, NSIS per-user,
/// deb/rpm/AppImage), and Desktop's own user-data directory as a last sign.
pub fn desktop_app() -> Option<PathBuf> {
    let home = dirs::home_dir();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/Applications/Cosmonic Desktop.app"));
        if let Some(h) = &home {
            candidates.push(h.join("Applications/Cosmonic Desktop.app"));
        }
    } else if cfg!(windows) {
        if let Some(d) = std::env::var_os("LOCALAPPDATA") {
            candidates
                .push(PathBuf::from(d).join("Programs/cosmonic-desktop/Cosmonic Desktop.exe"));
        }
        if let Some(d) = std::env::var_os("ProgramFiles") {
            candidates.push(PathBuf::from(d).join("Cosmonic Desktop/Cosmonic Desktop.exe"));
        }
    } else {
        candidates.extend(
            [
                "/opt/cosmonic-desktop/cosmonic-desktop",
                "/opt/Cosmonic Desktop/cosmonic-desktop",
                "/usr/bin/cosmonic-desktop",
                "/usr/share/applications/cosmonic-desktop.desktop",
            ]
            .map(PathBuf::from),
        );
        if let Some(h) = &home {
            candidates.push(h.join(".local/bin/cosmonic-desktop"));
            candidates.push(h.join(".local/share/applications/cosmonic-desktop.desktop"));
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Standard base64 (RFC 4648) with padding.
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (*chunk.get(1).unwrap_or(&0) as u32) << 8
            | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
    #[test]
    fn segments_follow_desktop_rules() {
        assert!(safe_segments("categories/mcp/weather/icon.svg").is_some());
        for bad in [
            "",
            "a//b",
            "../x",
            "a/../b",
            ".git/config",
            "a/.hidden",
            "a\\b",
        ] {
            assert!(safe_segments(bad).is_none(), "{bad}");
        }
    }
    #[test]
    fn jail_refuses_escapes() {
        let dir =
            std::env::temp_dir().join(format!("cosmonic-content-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("root/a")).unwrap();
        std::fs::write(dir.join("root/a/icon.svg"), "<svg/>").unwrap();
        std::fs::write(dir.join("outside.svg"), "<svg/>").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join("outside.svg"), dir.join("root/a/link.svg")).unwrap();
        let root = dir.join("root");
        assert!(jail(&root, "a/icon.svg").is_ok());
        #[cfg(unix)]
        assert!(jail(&root, "a/link.svg").is_err());
        assert!(jail(&root, "a/missing.svg").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
    #[test]
    fn repos_and_ids() {
        assert!(valid_repo("cosmonic-labs/launchpad"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("../x"));
        assert!(valid_id("my-source"));
        assert!(!valid_id("My Source"));
    }
}
