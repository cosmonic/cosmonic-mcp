//! Skills over MCP — the `io.modelcontextprotocol/skills` extension
//! (MCP 2026-07-28, [ext-skills]).
//!
//! The `cosmonic_*` tools are the hands; a **skill** is the manual. Without one,
//! a model faces 33 tools and has to infer the scaffold → build → publish →
//! apply → verify loop, the deny-all egress default, and the P2/P3 handler
//! distinction from tool descriptions alone. With one, it reads a playbook.
//!
//! Skills already ship as *installed* files — `~/.claude/skills/…` and the
//! per-agent equivalents, vendored per platform and kept current by
//! `src/main/agents.js`'s `autoUpdate()`. Serving them here means **connecting
//! to this server is enough**: every MCP client gets the playbook, including
//! the ones with no skills directory at all (LM Studio, NVIDIA OpenShell), and
//! an install that has gone stale is bypassed.
//!
//! ## What the extension defines, and what this module provides
//!
//! The extension is a transport binding for [Agent Skills] over the existing
//! **resources** primitive, plus three methods. This module is the data behind
//! all of them; `server.rs` is the dispatch.
//!
//! | Wire surface | Here |
//! |---|---|
//! | `capabilities.extensions["io.modelcontextprotocol/skills"]` | [`EXTENSION_ID`], [`capability`] |
//! | `skills/list` → `Skill[]` (frontmatter + manifest) | [`entries`] |
//! | `skills/get` → one `Skill` by its `SKILL.md` URI | [`entry`] |
//! | `resources/read` of `skill://<name>/<path>` | [`read`] |
//! | `resources/directory/read` (`directoryRead: true`) | [`directory`] |
//! | `resources/list` — one `SKILL.md` resource per skill | [`resources`] |
//! | `resources/templates/list` | [`resource_templates`] |
//!
//! A [`Skill` entry](https://github.com/modelcontextprotocol/ext-skills) is
//! the skill's `SKILL.md` URI, its YAML frontmatter **verbatim** as JSON, and
//! a complete manifest of every file with its SHA-256 digest and byte size. A
//! host builds its registry from entries alone, binds user approval to the
//! manifest, and verifies every byte it later reads against it. The entries
//! are computed once from the embedded bytes, so the manifest cannot disagree
//! with what `resources/read` serves.
//!
//! ## Progressive disclosure
//!
//! Discovery is three tiers, and the gap between them is the entire point:
//!
//! 1. The catalog — `skills/list`, or for a client without the extension the
//!    [`INDEX_URI`] resource (`skill://index.json`), which carries the same
//!    entries — read once at session start: names plus the trigger
//!    descriptions a client matches requests against. A few KB.
//! 2. Only when a request matches does it read `skill://<name>/SKILL.md` —
//!    the playbook, tens of KB.
//! 3. Relative paths inside a `SKILL.md` resolve against the skill root, so
//!    `references/templates.md` is `skill://<name>/references/templates.md`,
//!    pulled only at that depth.
//!
//! Reading all five playbooks and every reference would be ~300 KB. Reading
//! the catalog is a few KB.
//!
//! ## The legacy catalog
//!
//! [`INDEX_URI`] predates the extension and stays for clients that have not
//! adopted it (as of this writing that is every Claude surface — see the MCP
//! client matrix). It is a *mirror* of `skills/list`, not a second catalog:
//! its `skills` array is the same entries, so there is exactly one shape for a
//! skill on this server. A client with the extension never needs it, and the
//! spec is explicit that `skill://` resources are ordinary resources to a
//! client without one.
//!
//! ## Why `include_str!`
//!
//! The daemon runs as a service, with no dependency on the Electron app's
//! resource directory, and must work air-gapped. Embedding also makes drift
//! impossible: a playbook documenting a tool this binary does not have cannot
//! ship, because both come from the same build.
//!
//! ## Security
//!
//! [`read`], [`entry`] and [`directory`] resolve a URI by matching it
//! **verbatim** against the static [`SKILLS`] table. There is no filesystem
//! lookup, so there is no path traversal to defend against —
//! `skill://x/../../etc/passwd` simply matches nothing and returns `None`. Keep
//! it that way: if this ever reads from disk, it needs a canonicalize-and-
//! contain check first.
//!
//! Digests are what the spec says they are: consistency between the manifest
//! and the bytes, not a trust anchor. Both come from this binary.
//!
//! ## Adding or updating a skill
//!
//! Don't edit `skills/` or the generated table by hand. `agent-integrations`
//! is the source of truth; run `node scripts/vendor-daemon-skills.mjs` and
//! commit the diff.
//!
//! [ext-skills]: https://github.com/modelcontextprotocol/ext-skills
//! [Agent Skills]: https://agentskills.io/specification

use std::sync::OnceLock;

use rmcp::model::{JsonObject, MetaObject, Resource, ResourceTemplate};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// Identifier of the MCP skills extension this module implements.
pub const EXTENSION_ID: &str = "io.modelcontextprotocol/skills";

/// The `skills/list` method, defined by the extension.
pub const LIST_METHOD: &str = "skills/list";

/// The `skills/get` method, defined by the extension.
pub const GET_METHOD: &str = "skills/get";

/// The optional `resources/directory/read` method, gated on `directoryRead`.
pub const DIRECTORY_READ_METHOD: &str = "resources/directory/read";

/// `mimeType` of a directory resource, per the extension.
pub const DIRECTORY_MIME: &str = "inode/directory";

/// The `_meta` key prefix reserved for skill resources by the extension.
const META_PREFIX: &str = "io.modelcontextprotocol.skills/";

/// URI of the legacy skill catalog — a mirror of `skills/list` for clients
/// without the extension. See the module docs.
pub const INDEX_URI: &str = "skill://index.json";

/// Scheme prefix for every skill resource URI.
const SCHEME: &str = "skill://";

/// The entrypoint filename of a skill bundle, per the Agent Skills spec.
const ENTRY: &str = "SKILL.md";

/// A supporting file inside a skill bundle, served at `skill://<skill>/<path>`.
pub struct SkillFile {
    /// Path relative to the skill root, e.g. `references/templates.md`.
    /// Matched verbatim against the requested URI.
    pub path: &'static str,
    /// File contents, embedded at compile time.
    pub text: &'static str,
    /// MIME type reported to the client.
    pub mime_type: &'static str,
}

/// One skill: a `SKILL.md` playbook and its supporting files.
pub struct Skill {
    /// Skill name; the authority segment of its URIs, the name the same skill
    /// has when installed on disk, and — the spec requires — the `name` in its
    /// frontmatter.
    pub name: &'static str,
    /// The playbook, served at `skill://<name>/SKILL.md`. Its own field rather
    /// than one of `files` so a skill structurally cannot exist without it.
    pub skill_md: &'static str,
    /// Supporting files, if any.
    pub files: &'static [SkillFile],
}

/// The version of `agent-integrations` these skills were vendored from.
// BEGIN SKILLS (generated by scripts/vendor-daemon-skills.mjs — do not edit by hand)
pub const SKILLS_VERSION: &str = "1.1.0";

pub static SKILLS: &[Skill] = &[
    Skill {
        name: "cosmonic-sandbox",
        skill_md: include_str!("../skills/cosmonic-sandbox/SKILL.md"),
        files: &[
            SkillFile {
                path: "references/composition.md",
                text: include_str!("../skills/cosmonic-sandbox/references/composition.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/crds.md",
                text: include_str!("../skills/cosmonic-sandbox/references/crds.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/http-trigger.md",
                text: include_str!("../skills/cosmonic-sandbox/references/http-trigger.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/local-ai.md",
                text: include_str!("../skills/cosmonic-sandbox/references/local-ai.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/mcp-servers.md",
                text: include_str!("../skills/cosmonic-sandbox/references/mcp-servers.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/oci-registry.md",
                text: include_str!("../skills/cosmonic-sandbox/references/oci-registry.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/patterns-and-smoke-tests.md",
                text: include_str!(
                    "../skills/cosmonic-sandbox/references/patterns-and-smoke-tests.md"
                ),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/recipes.md",
                text: include_str!("../skills/cosmonic-sandbox/references/recipes.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/templates.md",
                text: include_str!("../skills/cosmonic-sandbox/references/templates.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/wasm-crate-compat.md",
                text: include_str!("../skills/cosmonic-sandbox/references/wasm-crate-compat.md"),
                mime_type: "text/markdown",
            },
        ],
    },
    Skill {
        name: "cosmonic-go",
        skill_md: include_str!("../skills/cosmonic-go/SKILL.md"),
        files: &[SkillFile {
            path: "references/upstream.md",
            text: include_str!("../skills/cosmonic-go/references/upstream.md"),
            mime_type: "text/markdown",
        }],
    },
    Skill {
        name: "cosmonic-nats",
        skill_md: include_str!("../skills/cosmonic-nats/SKILL.md"),
        files: &[
            SkillFile {
                path: "references/nats-concepts.md",
                text: include_str!("../skills/cosmonic-nats/references/nats-concepts.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/nats-jetstream.md",
                text: include_str!("../skills/cosmonic-nats/references/nats-jetstream.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/nats-security.md",
                text: include_str!("../skills/cosmonic-nats/references/nats-security.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/nats-server.md",
                text: include_str!("../skills/cosmonic-nats/references/nats-server.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/patterns.md",
                text: include_str!("../skills/cosmonic-nats/references/patterns.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/wasmcloud-nats.md",
                text: include_str!("../skills/cosmonic-nats/references/wasmcloud-nats.md"),
                mime_type: "text/markdown",
            },
        ],
    },
    Skill {
        name: "cosmonic-nats-tuning",
        skill_md: include_str!("../skills/cosmonic-nats-tuning/SKILL.md"),
        files: &[SkillFile {
            path: "references/nats-tuning.md",
            text: include_str!("../skills/cosmonic-nats-tuning/references/nats-tuning.md"),
            mime_type: "text/markdown",
        }],
    },
    Skill {
        name: "cosmonic-kafka",
        skill_md: include_str!("../skills/cosmonic-kafka/SKILL.md"),
        files: &[
            SkillFile {
                path: "references/cosmonic-kafka.md",
                text: include_str!("../skills/cosmonic-kafka/references/cosmonic-kafka.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/patterns.md",
                text: include_str!("../skills/cosmonic-kafka/references/patterns.md"),
                mime_type: "text/markdown",
            },
            SkillFile {
                path: "references/troubleshooting.md",
                text: include_str!("../skills/cosmonic-kafka/references/troubleshooting.md"),
                mime_type: "text/markdown",
            },
        ],
    },
];
// END SKILLS

impl Skill {
    /// URI of this skill's `SKILL.md` — the skill's identity on this server.
    pub fn entry_uri(&self) -> String {
        format!("{SCHEME}{}/{ENTRY}", self.name)
    }

    /// URI of the skill's root directory: the `SKILL.md` URI with `/SKILL.md`
    /// removed and no trailing slash, per the extension.
    pub fn root_uri(&self) -> String {
        format!("{SCHEME}{}", self.name)
    }

    /// URI of a supporting file.
    pub fn file_uri(&self, file: &SkillFile) -> String {
        format!("{SCHEME}{}/{}", self.name, file.path)
    }

    /// The `SKILL.md` YAML frontmatter, verbatim, as a JSON object.
    ///
    /// Parsed once per process. The Agent Skills spec requires `name` and
    /// `description`; a `SKILL.md` whose frontmatter does not parse, or lacks
    /// either, yields an object with only what could be recovered — and the
    /// tests below refuse to let such a skill ship.
    pub fn frontmatter(&self) -> &'static Map<String, Value> {
        &self.entry_parts().0
    }

    /// The `description` from the frontmatter — the trigger text a client
    /// matches user requests against.
    pub fn description(&self) -> &'static str {
        self.frontmatter()
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    /// The spec's `Skill` entry: `{uri, frontmatter, resources}`, with a
    /// complete manifest — `SKILL.md` first, then every supporting file, each
    /// with the SHA-256 digest and byte size of exactly what [`read`] serves.
    pub fn entry(&self) -> &'static Value {
        &self.entry_parts().1
    }

    /// Byte size of every file in the manifest — the extension caps a skill at
    /// 16 MiB and 512 files, and both are checkable from the entry alone.
    #[cfg(test)]
    fn total_bytes(&self) -> usize {
        self.skill_md.len() + self.files.iter().map(|f| f.text.len()).sum::<usize>()
    }

    fn entry_parts(&self) -> &'static (Map<String, Value>, Value) {
        // One cell for the whole table rather than one per skill: `SKILLS` is a
        // static array of a plain struct, and a `OnceLock` field would have to
        // be emitted by the generator.
        static ENTRIES: OnceLock<Vec<(Map<String, Value>, Value)>> = OnceLock::new();
        let entries = ENTRIES.get_or_init(|| SKILLS.iter().map(Skill::build_entry).collect());
        let index = SKILLS
            .iter()
            .position(|s| std::ptr::eq(s, self))
            .expect("Skill::entry called on a skill outside SKILLS");
        &entries[index]
    }

    fn build_entry(&self) -> (Map<String, Value>, Value) {
        let frontmatter = parse_frontmatter(self.skill_md).unwrap_or_default();
        let mut resources = Vec::with_capacity(self.files.len() + 1);
        resources.push(json!({
            "uri": self.entry_uri(),
            "digest": digest(self.skill_md),
            "size": self.skill_md.len(),
        }));
        for file in self.files {
            resources.push(json!({
                "uri": self.file_uri(file),
                "digest": digest(file.text),
                "size": file.text.len(),
            }));
        }
        let entry = json!({
            "uri": self.entry_uri(),
            "frontmatter": frontmatter,
            "resources": resources,
        });
        (frontmatter, entry)
    }
}

/// The extension's per-server settings, as declared under
/// `capabilities.extensions[EXTENSION_ID]`.
///
/// `directoryRead: true` is a promise to answer `resources/directory/read`
/// for every directory in the skill namespace; [`directory`] keeps it.
pub fn capability() -> JsonObject {
    let mut settings = JsonObject::new();
    settings.insert("directoryRead".into(), Value::Bool(true));
    settings
}

/// Every skill's entry, in catalog order — the `skills` array of
/// `skills/list`. The listing is complete and never paginated: five entries
/// with their manifests are a few KB, and the spec forbids splitting an entry
/// across pages anyway.
pub fn entries() -> Vec<Value> {
    SKILLS.iter().map(|s| s.entry().clone()).collect()
}

/// The entry for the skill whose `SKILL.md` URI this is — `skills/get`.
///
/// `None` when this server serves no skill there, which the caller turns into
/// `-32602`. Verbatim match: only the exact `SKILL.md` URI names a skill, so
/// a root URI, a supporting file, or a traversal-shaped path all miss.
pub fn entry(uri: &str) -> Option<&'static Value> {
    let (name, path) = uri.strip_prefix(SCHEME)?.split_once('/')?;
    if path != ENTRY {
        return None;
    }
    SKILLS.iter().find(|s| s.name == name).map(Skill::entry)
}

/// The legacy catalog served at [`INDEX_URI`] — the same entries
/// `skills/list` returns, wrapped for a client that reads it as a resource.
///
/// Built from [`SKILLS`] on every read rather than stored, so it cannot fall
/// out of step with the embedded files.
pub fn index_json() -> String {
    let index = json!({
        "schemaVersion": "2",
        "extension": EXTENSION_ID,
        "server": {
            "name": "cosmonic-desktop",
            "version": env!("CARGO_PKG_VERSION"),
            "skillsVersion": SKILLS_VERSION,
        },
        "usage": "A mirror of this server's `skills/list` for clients without the \
                  io.modelcontextprotocol/skills extension. Each entry is one skill: its \
                  SKILL.md `uri`, its `frontmatter` (the `description` is what a task is \
                  matched against), and `resources` — every file of the skill with its \
                  SHA-256 digest and size. A playbook names its supporting files by path \
                  relative to the skill root (for example `references/recipes.md`); that \
                  file is skill://<skill>/<that path>, and every such URI is in `resources`.",
        "skills": entries(),
    });
    // Built from &'static str and owned values, so serialization cannot fail —
    // but never panic in the daemon over a resource read.
    serde_json::to_string_pretty(&index).unwrap_or_else(|_| String::from(r#"{"skills":[]}"#))
}

/// The skill resources for `resources/list`: the legacy catalog and one
/// `SKILL.md` per skill, with the metadata the extension prescribes for it —
/// `name` and `description` from the frontmatter, `text/markdown`, and the
/// remaining frontmatter fields under the reserved `_meta` prefix.
///
/// Supporting files are deliberately NOT listed. They are enumerated, with
/// digests, in the skill's manifest (`skills/list`, `skills/get`, the
/// catalog), reachable through `resources/directory/read`, and readable by
/// URI; listing all of them here buried the five playbooks in twenty-odd
/// reference files and gave a client nothing it did not already have.
pub fn resources() -> Vec<Resource> {
    let mut out = vec![Resource::new(INDEX_URI, "skill-index")
        .with_title("Skill catalog")
        .with_description(
            "The skills this server publishes — the same entries as `skills/list`, for a \
             client without the io.modelcontextprotocol/skills extension: each skill's \
             SKILL.md URI, its frontmatter (name and trigger description), and its file \
             manifest with SHA-256 digests.",
        )
        .with_mime_type("application/json")];

    for skill in SKILLS {
        let mut meta = JsonObject::new();
        for (key, value) in skill.frontmatter() {
            if key != "name" && key != "description" {
                meta.insert(format!("{META_PREFIX}{key}"), value.clone());
            }
        }
        let mut resource = Resource::new(skill.entry_uri(), skill.name)
            .with_title(format!("{} skill", skill.name))
            .with_description(skill.description())
            .with_mime_type("text/markdown")
            .with_size(clamp_size(skill.skill_md.len()));
        if !meta.is_empty() {
            resource = resource.with_meta(MetaObject::from(meta));
        }
        out.push(resource);
    }
    out
}

/// RFC 6570 templates for `resources/templates/list`, so a client can build a
/// skill URI without enumerating every resource.
pub fn resource_templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("skill://{skill}/SKILL.md", "skill-playbook")
            .with_title("Skill playbook")
            .with_description(
                "The SKILL.md of a named skill. Skill names come from skills/list (or the \
                 skill://index.json catalog).",
            )
            .with_mime_type("text/markdown"),
        ResourceTemplate::new("skill://{skill}/{+path}", "skill-file")
            .with_title("Skill supporting file")
            .with_description(
                "A file bundled with a skill, at the path its SKILL.md names relative to the \
                 skill root. Every such URI is in the skill's manifest.",
            ),
    ]
}

/// Resolve a `skill://` URI to `(mime_type, contents)`, or `None` when this
/// server serves nothing there.
///
/// Verbatim table lookup — see the module docs on why that is the security
/// property, not an implementation detail.
pub fn read(uri: &str) -> Option<(&'static str, String)> {
    if uri == INDEX_URI {
        return Some(("application/json", index_json()));
    }
    let (name, path) = uri.strip_prefix(SCHEME)?.split_once('/')?;
    let skill = SKILLS.iter().find(|skill| skill.name == name)?;
    if path == ENTRY {
        return Some(("text/markdown", skill.skill_md.to_owned()));
    }
    let file = skill.files.iter().find(|file| file.path == path)?;
    Some((file.mime_type, file.text.to_owned()))
}

/// The direct children of a directory resource — `resources/directory/read`.
///
/// `None` when the URI is not a directory this server serves (a file, an
/// unknown skill, a trailing slash, a traversal), which the caller turns into
/// `-32602`. Directories are implied by the file paths: `skill://<name>` is
/// every skill's root and `skill://<name>/references` exists because files
/// live under it. Files carry their ordinary resource metadata; a
/// subdirectory is listed once, as `inode/directory`.
pub fn directory(uri: &str) -> Option<Vec<Resource>> {
    let rest = uri.strip_prefix(SCHEME)?;
    let (name, dir) = match rest.split_once('/') {
        Some((name, dir)) => (name, Some(dir)),
        None => (rest, None),
    };
    if name.is_empty() {
        return None;
    }
    let skill = SKILLS.iter().find(|skill| skill.name == name)?;

    // Every file path relative to the skill root, with its resource metadata,
    // in a fixed order: the playbook, then the supporting files as generated.
    let files = std::iter::once((ENTRY, "text/markdown", skill.skill_md.len())).chain(
        skill
            .files
            .iter()
            .map(|f| (f.path, f.mime_type, f.text.len())),
    );

    // The root is `dir == ""`; below it every segment must be a real name —
    // no empty (`//`, a trailing slash), `.` or `..` segments, which is the
    // same verbatim discipline `read` has: a shaped path matches nothing.
    let dir = match dir {
        None => "",
        Some("") => return None, // `skill://<name>/` — a trailing slash is not the root
        Some(dir) => dir,
    };
    if !dir.is_empty()
        && dir
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return None;
    }
    let mut out: Vec<Resource> = Vec::new();
    let mut subdirs: Vec<&str> = Vec::new();
    let mut is_dir = dir.is_empty();
    for (path, mime, len) in files {
        let below = if dir.is_empty() {
            path
        } else {
            let Some(below) = path.strip_prefix(dir) else {
                continue;
            };
            let Some(below) = below.strip_prefix('/') else {
                continue;
            };
            below
        };
        is_dir = true;
        match below.split_once('/') {
            None => out.push(
                Resource::new(format!("{SCHEME}{name}/{path}"), below)
                    .with_mime_type(mime)
                    .with_size(clamp_size(len)),
            ),
            Some((child, _)) => {
                if !subdirs.contains(&child) {
                    subdirs.push(child);
                    let child_path = if dir.is_empty() {
                        child.to_string()
                    } else {
                        format!("{dir}/{child}")
                    };
                    out.push(
                        Resource::new(format!("{SCHEME}{name}/{child_path}"), child)
                            .with_mime_type(DIRECTORY_MIME),
                    );
                }
            }
        }
    }
    is_dir.then_some(out)
}

/// `sha256:<64 lowercase hex>` of the raw bytes, the extension's digest form.
fn digest(text: &str) -> String {
    let mut out = String::with_capacity("sha256:".len() + 64);
    out.push_str("sha256:");
    for byte in Sha256::digest(text.as_bytes()) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `Resource.size` is advisory; saturate rather than panic on a hypothetical
/// usize wider than u64.
fn clamp_size(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// The YAML frontmatter of a Markdown file as a JSON object, verbatim.
///
/// The extension requires the entry's `frontmatter` to be "identical in
/// content" to the file's, field by field, and a host verifies exactly that
/// after it reads the playbook — so this is a real YAML parse, not the
/// two-field line scan it replaced. `None` when there is no frontmatter block
/// or it is not a mapping.
///
/// CRLF-tolerant: git checks these files out with CRLF on Windows, and a
/// parser that only knew `---\n` once returned nothing for every field there,
/// which published five skills no request could match.
fn parse_frontmatter(markdown: &str) -> Option<Map<String, Value>> {
    let body = markdown
        .strip_prefix("---\n")
        .or_else(|| markdown.strip_prefix("---\r\n"))?;
    let end = body.find("\n---")?;
    let block = &body[..end];
    let yaml: serde_yaml::Value = serde_yaml::from_str(block).ok()?;
    match serde_json::to_value(yaml).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The extension's per-skill ceilings; a host MUST accept up to these and
    /// a server SHOULD NOT exceed them.
    const MAX_FILES: usize = 512;
    const MAX_BYTES: usize = 16 * 1024 * 1024;

    #[test]
    fn every_skill_has_a_playbook_and_a_trigger_description() {
        assert!(!SKILLS.is_empty(), "the vendor step produced no skills");
        for skill in SKILLS {
            assert!(
                skill.skill_md.starts_with("---\n") || skill.skill_md.starts_with("---\r\n"),
                "{}: SKILL.md has no frontmatter",
                skill.name
            );
            // The description IS the trigger: an empty one publishes a skill no
            // model can ever match, which looks like the feature working.
            let description = skill.description();
            assert!(
                description.len() > 40,
                "{}: description is {} chars — too short to match a request on",
                skill.name,
                description.len()
            );
        }
    }

    #[test]
    fn skill_names_are_unique() {
        // `read()` resolves by first match, so a duplicate name would shadow a
        // whole skill's files with no error anywhere.
        let mut names: Vec<_> = SKILLS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate skill name in SKILLS");
    }

    // ---- the Skill entry (ext-skills "Skill Entries") -------------------

    /// The Agent Skills rule the extension leans on: the final `<skill-path>`
    /// segment of the entry's `uri` MUST equal `frontmatter.name`, so the name
    /// is recoverable from the URI alone.
    #[test]
    fn frontmatter_name_matches_the_uri_segment() {
        for skill in SKILLS {
            let entry = skill.entry();
            assert_eq!(entry["uri"], Value::from(skill.entry_uri()));
            assert_eq!(
                entry["frontmatter"]["name"],
                Value::from(skill.name),
                "{}: frontmatter.name disagrees with the directory",
                skill.name
            );
            assert_eq!(
                entry["frontmatter"]["description"],
                Value::from(skill.description())
            );
        }
    }

    /// `frontmatter` is the whole block, verbatim — not a curated subset. A
    /// host compares it field-by-field with what it parses from SKILL.md, so a
    /// field this drops fails verification on the host.
    #[test]
    fn frontmatter_is_verbatim_not_a_two_field_subset() {
        for skill in SKILLS {
            let fm = skill.frontmatter();
            // Every vendored skill declares these beyond name/description.
            for key in ["license", "compatibility", "metadata"] {
                assert!(
                    fm.contains_key(key),
                    "{}: frontmatter lost `{key}`",
                    skill.name
                );
            }
            assert!(
                fm["metadata"].get("version").is_some(),
                "{}: nested metadata.version lost",
                skill.name
            );
            // Nothing outside the block leaks in.
            assert!(!fm.contains_key("body"));
        }
    }

    /// The YAML parse must agree with the single-line scan the vendor script
    /// validates against: a description with a ` #` or a `: ` in it would
    /// parse differently, and the catalog would silently carry a truncated
    /// trigger.
    #[test]
    fn parsed_description_is_the_whole_single_line_field() {
        for skill in SKILLS {
            let body = skill.skill_md.strip_prefix("---\n").unwrap();
            let raw = body[..body.find("\n---").unwrap()]
                .lines()
                .find_map(|l| l.strip_prefix("description:"))
                .map(str::trim)
                .unwrap_or("");
            assert_eq!(
                skill.description(),
                raw,
                "{}: YAML parse changed the description",
                skill.name
            );
        }
    }

    /// The manifest is complete, lists SKILL.md first, and every digest and
    /// size describes exactly the bytes `resources/read` returns — the
    /// property a host verifies on every read.
    #[test]
    fn manifest_digests_and_sizes_match_what_read_serves() {
        for skill in SKILLS {
            let resources = skill.entry()["resources"]
                .as_array()
                .expect("array manifest");
            assert_eq!(
                resources.len(),
                skill.files.len() + 1,
                "{}: incomplete",
                skill.name
            );
            assert_eq!(resources[0]["uri"], Value::from(skill.entry_uri()));
            for r in resources {
                let uri = r["uri"].as_str().unwrap();
                let (_, body) =
                    read(uri).unwrap_or_else(|| panic!("manifest lists unreadable {uri}"));
                assert_eq!(
                    r["size"].as_u64().unwrap(),
                    body.len() as u64,
                    "{uri}: size"
                );
                let d = r["digest"].as_str().unwrap();
                assert_eq!(d, digest(&body), "{uri}: digest");
                let hex = d.strip_prefix("sha256:").expect("sha256: prefix");
                assert_eq!(hex.len(), 64);
                assert!(
                    hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
                    "{uri}: not lowercase hex"
                );
            }
            // Each file exactly once.
            let mut uris: Vec<_> = resources
                .iter()
                .map(|r| r["uri"].as_str().unwrap())
                .collect();
            uris.sort_unstable();
            let n = uris.len();
            uris.dedup();
            assert_eq!(n, uris.len(), "{}: duplicate manifest entry", skill.name);
        }
    }

    #[test]
    fn digest_is_the_spec_form() {
        // The spec's own worked example: an empty input, for a fixed answer.
        assert_eq!(
            digest(""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn every_skill_is_within_the_extension_limits() {
        for skill in SKILLS {
            assert!(
                skill.files.len() < MAX_FILES,
                "{}: too many files",
                skill.name
            );
            assert!(
                skill.total_bytes() <= MAX_BYTES,
                "{}: over 16 MiB",
                skill.name
            );
        }
    }

    // ---- skills/list and skills/get ----------------------------------

    #[test]
    fn entries_is_one_complete_entry_per_skill() {
        let list = entries();
        assert_eq!(list.len(), SKILLS.len());
        for (skill, entry) in SKILLS.iter().zip(&list) {
            assert_eq!(entry, skill.entry());
            assert!(
                entry["resources"].is_array(),
                "{}: resources must be an array, never absent",
                skill.name
            );
        }
    }

    #[test]
    fn get_resolves_only_the_exact_skill_md_uri() {
        for skill in SKILLS {
            assert_eq!(entry(&skill.entry_uri()), Some(skill.entry()));
            // Not the root, not a supporting file, not a near-miss.
            assert!(entry(&skill.root_uri()).is_none());
            assert!(entry(&format!("{}/", skill.entry_uri())).is_none());
            assert!(entry(&format!("skill://{}/skill.md", skill.name)).is_none());
            for file in skill.files {
                assert!(entry(&skill.file_uri(file)).is_none());
            }
        }
        assert!(entry(INDEX_URI).is_none());
        assert!(entry("skill://nope/SKILL.md").is_none());
        assert!(entry("skill://../SKILL.md").is_none());
        assert!(entry("").is_none());
    }

    #[test]
    fn capability_declares_directory_read() {
        let cap = capability();
        assert_eq!(cap.get("directoryRead"), Some(&Value::Bool(true)));
    }

    // ---- the legacy catalog --------------------------------------------

    #[test]
    fn the_index_is_small_enough_to_read_at_session_start() {
        // The whole point of progressive disclosure: the catalog is read on
        // EVERY session, the playbooks only on a match. If the index ever
        // approaches the size of the bodies it indexes, the tiering is gone.
        let index = index_json();
        let bodies: usize = SKILLS.iter().map(Skill::total_bytes).sum();
        assert!(
            index.len() < 24 * 1024,
            "skill index is {} bytes; keep it readable at session start",
            index.len()
        );
        assert!(
            index.len() * 8 < bodies,
            "index ({}) is not much cheaper than reading everything ({bodies})",
            index.len()
        );
    }

    #[test]
    fn the_index_mirrors_skills_list() {
        // One shape for a skill on this server: a client that reads the
        // catalog as a resource sees exactly what skills/list returns.
        let index: Value = serde_json::from_str(&index_json()).expect("index is JSON");
        assert_eq!(index["extension"], Value::from(EXTENSION_ID));
        assert_eq!(index["skills"], Value::Array(entries()));
        for entry in index["skills"].as_array().unwrap() {
            let uri = entry["uri"].as_str().unwrap();
            assert!(read(uri).is_some(), "index advertises unreadable {uri}");
            for file in entry["resources"].as_array().unwrap() {
                let uri = file["uri"].as_str().unwrap();
                assert!(read(uri).is_some(), "index advertises unreadable {uri}");
            }
        }
    }

    // ---- resources/list ------------------------------------------------

    #[test]
    fn every_listed_resource_reads_and_carries_the_prescribed_metadata() {
        let listed = resources();
        // The catalog + one playbook per skill — and nothing else. Supporting
        // files are the manifest's job.
        assert_eq!(
            listed.len(),
            SKILLS.len() + 1,
            "resources() lists more than the playbooks"
        );
        for resource in &listed {
            let uri = &resource.uri;
            let (mime, body) = read(uri).unwrap_or_else(|| panic!("listed but unreadable: {uri}"));
            assert!(!body.is_empty(), "{uri} is empty");
            // resources/list must report the same type resources/read returns,
            // or a client that keys off it re-parses the body.
            assert_eq!(
                resource.mime_type.as_deref(),
                Some(mime),
                "mime mismatch on {uri}"
            );
        }
        for skill in SKILLS {
            let r = listed
                .iter()
                .find(|r| r.uri == skill.entry_uri())
                .expect("playbook listed");
            assert_eq!(r.name, skill.name, "name SHOULD be the frontmatter name");
            assert_eq!(r.description.as_deref(), Some(skill.description()));
            assert_eq!(r.size, Some(skill.skill_md.len() as u64));
            let meta = r.meta.as_ref().expect("extra frontmatter under _meta");
            assert!(
                meta.keys().all(|k| k.starts_with(META_PREFIX)),
                "{}: unprefixed _meta key",
                skill.name
            );
            assert!(meta.contains_key(&format!("{META_PREFIX}license")));
            assert!(
                !meta.contains_key(&format!("{META_PREFIX}name")),
                "name belongs on the resource"
            );
        }
    }

    // ---- resources/directory/read ---------------------------------------

    #[test]
    fn directory_read_lists_direct_children_only() {
        let skill = SKILLS
            .iter()
            .find(|s| !s.files.is_empty())
            .expect("a skill with references");
        let root = directory(&skill.root_uri()).expect("root is a directory");
        let names: Vec<_> = root.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"SKILL.md"), "{names:?}");
        assert!(names.contains(&"references"), "{names:?}");
        // The subdirectory is listed once, as a directory, with no trailing slash.
        let refs: Vec<_> = root.iter().filter(|r| r.name == "references").collect();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].mime_type.as_deref(), Some(DIRECTORY_MIME));
        assert_eq!(refs[0].uri, format!("{}/references", skill.root_uri()));
        // Nothing from below `references/` leaks into the root listing.
        assert!(!names.iter().any(|n| n.contains('/')), "{names:?}");
        assert_eq!(root.len(), 2);

        let below = directory(&refs[0].uri).expect("references is a directory");
        assert_eq!(below.len(), skill.files.len());
        for (file, resource) in skill.files.iter().zip(&below) {
            assert_eq!(resource.uri, skill.file_uri(file));
            assert_eq!(resource.name, file.path.rsplit('/').next().unwrap());
            assert_eq!(resource.mime_type.as_deref(), Some(file.mime_type));
            assert!(read(&resource.uri).is_some());
        }
    }

    #[test]
    fn every_directory_in_the_namespace_answers() {
        // `directoryRead: true` promises the method for EVERY directory in the
        // skill namespaces served as files — walk the whole tree.
        for skill in SKILLS {
            let mut pending = vec![skill.root_uri()];
            let mut seen_files = 0usize;
            while let Some(dir) = pending.pop() {
                for child in directory(&dir).unwrap_or_else(|| panic!("{dir} is not a directory")) {
                    if child.mime_type.as_deref() == Some(DIRECTORY_MIME) {
                        pending.push(child.uri);
                    } else {
                        seen_files += 1;
                    }
                }
            }
            assert_eq!(
                seen_files,
                skill.files.len() + 1,
                "{}: walk missed files",
                skill.name
            );
        }
    }

    #[test]
    fn directory_read_refuses_files_unknowns_and_traversal() {
        let skill = SKILLS.iter().find(|s| !s.files.is_empty()).unwrap();
        let name = skill.name;
        for uri in [
            skill.entry_uri(),                     // a file, not a directory
            skill.file_uri(&skill.files[0]),       // a file
            format!("skill://{name}/"),            // trailing slash
            format!("skill://{name}/references/"), // trailing slash
            format!("skill://{name}/nope"),        // no such directory
            format!("skill://{name}/../{name}"),   // traversal
            format!("skill://{name}/./references"),
            format!("skill://{name}//references"),
            "skill://nope".into(),
            "skill://".into(),
            INDEX_URI.into(),
            "cosmonic://host".into(),
            String::new(),
        ] {
            assert!(directory(&uri).is_none(), "{uri:?} read as a directory");
        }
    }

    /// Relative Markdown links inside a SKILL.md become `skill://` URIs — that
    /// is what makes the third disclosure tier work. A link to a file we did
    /// not embed is a resource the model will ask for and not get, and it fails
    /// silently: the model just proceeds without the reference it wanted.
    ///
    /// `scripts/vendor-daemon-skills.mjs` refuses to vendor a dangling link;
    /// this is the same check on the compiled-in copy, so hand-editing
    /// `skills/` cannot bypass it either.
    /// Reference paths a playbook names that this server does not serve.
    ///
    /// Resolution is CROSS-SKILL on purpose: `cosmonic-go` says "see
    /// `cosmonic-sandbox` `references/composition.md`", and that file is served
    /// — under another skill. Checking only the naming skill would call a
    /// working reference broken.
    ///
    /// Only skill-bundle paths count. `references/…` and `…/SKILL.md` are ours;
    /// a path like `docs/tuning.md` in "each scaffold's `docs/tuning.md`" is a
    /// file in the USER's generated project, which no MCP resource ever backs.
    fn unserved_references() -> Vec<String> {
        let mut out = Vec::new();
        for skill in SKILLS {
            for target in referenced_files(skill.skill_md) {
                if target.contains(':') || target.starts_with('/') {
                    continue; // absolute or external — not ours to serve
                }
                let path = target.trim_start_matches("./");
                let bundle_path = path.starts_with("references/") || path.ends_with("SKILL.md");
                if !bundle_path {
                    continue;
                }
                let served = SKILLS
                    .iter()
                    .any(|s| read(&format!("{SCHEME}{}/{path}", s.name)).is_some());
                if !served {
                    out.push(format!("{}: {path}", skill.name));
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Paths a playbook names that this server cannot serve — a ratchet, not a
    /// suppression. Each entry is a real gap in the MCP surface: a model told to
    /// read the file gets nothing back, silently, and carries on without it.
    ///
    /// Fixes belong upstream in `agent-integrations` (issue #501 phase 7); when
    /// one lands and is re-vendored, this test FAILS until the entry is removed,
    /// so the list can only ever shrink.
    const KNOWN_UNSERVED: &[&str] = &[
        // The authoring guide for building MCP servers is not part of the
        // client-facing skill set, so nothing serves it over MCP. Upstream
        // should fold what a deploying agent needs into
        // cosmonic-sandbox/references/mcp-servers.md and drop the pointer.
        "cosmonic-sandbox: skills/building-mcp-servers/SKILL.md",
    ];

    #[test]
    fn no_skill_points_at_a_file_that_is_not_served() {
        // Relative paths inside a playbook become skill:// URIs — that is what
        // makes the third disclosure tier work. A path we do not serve fails
        // SILENTLY: the model asks, gets nothing, and proceeds without the
        // detail it wanted.
        let unserved = unserved_references();
        let unexpected: Vec<_> = unserved
            .iter()
            .filter(|u| !KNOWN_UNSERVED.contains(&u.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "playbooks point at files this server does not serve: {unexpected:?}"
        );
        let fixed: Vec<_> = KNOWN_UNSERVED
            .iter()
            .filter(|k| !unserved.iter().any(|u| u == *k))
            .collect();
        assert!(
            fixed.is_empty(),
            "these are served now — drop them from KNOWN_UNSERVED: {fixed:?}"
        );
    }

    #[test]
    fn the_reference_check_actually_examines_references() {
        // Without this the suite passes when `referenced_files` stops matching.
        // It did: the first version understood only `[text](path.md)` links and
        // examined ZERO paths across all the skills, because the playbooks name
        // their files as backticked paths instead.
        let examined: usize = SKILLS
            .iter()
            .map(|s| referenced_files(s.skill_md).len())
            .sum();
        assert!(
            examined >= 4 * SKILLS.len(),
            "only {examined} reference paths found across {} skills; the extractor is broken",
            SKILLS.len()
        );
        // And it must resolve most of them, or "serving skills" is a fiction.
        assert!(
            unserved_references().len() <= KNOWN_UNSERVED.len(),
            "more unserved references than the known set"
        );
    }

    /// Every `.md` file a playbook points a reader at.
    ///
    /// Two forms, because the skills use both. Markdown links —
    /// `[Recipes](references/recipes.md)` — and, far more often, a backticked
    /// path written as if on disk: `` `references/recipes.md` ``. The second is
    /// the Agent Skills convention (an installed skill IS a directory), and it
    /// is why a link check that only understood `](...)` silently examined
    /// ZERO links across all the skills while passing.
    fn referenced_files(markdown: &str) -> Vec<String> {
        let mut out = Vec::new();

        let mut rest = markdown;
        while let Some(open) = rest.find("](") {
            rest = &rest[open + 2..];
            let Some(close) = rest.find(')') else { break };
            let target = rest[..close].split(['#', ' ']).next().unwrap_or("");
            if target.ends_with(".md") {
                out.push(target.to_string());
            }
            rest = &rest[close..];
        }

        for span in markdown.split('`').skip(1).step_by(2) {
            // A backtick span is a path only if it is one: no spaces, ends .md.
            let target = span.split(['#', ' ']).next().unwrap_or("");
            if target.ends_with(".md") && !target.contains(char::is_whitespace) {
                out.push(target.to_string());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// The security property: `read` is a verbatim lookup in a static table.
    /// There is no filesystem call, so a traversal-shaped URI matches nothing.
    /// If this ever reads from disk it needs a canonicalize-and-contain check
    /// FIRST — these cases are the tripwire for that change.
    #[test]
    fn read_resolves_nothing_outside_the_static_table() {
        let name = SKILLS[0].name;
        for uri in [
            &format!("skill://{name}/../../../etc/passwd"),
            &format!("skill://{name}/./SKILL.md"),
            &format!("skill://{name}/references/../SKILL.md"),
            &format!("skill://{name}/SKILL.md/"),
            &format!("skill://{name}//SKILL.md"),
            &format!("skill://{name}/skill.md"), // case matters
            "skill://../SKILL.md",
            "skill://index.json/../SKILL.md",
            "skill:///SKILL.md",
            "file:///etc/passwd",
            "skill://",
            "",
        ] {
            assert!(read(uri).is_none(), "resolved something for {uri:?}");
        }
    }

    #[test]
    fn read_serves_the_index_and_each_playbook() {
        let (mime, body) = read(INDEX_URI).expect("index");
        assert_eq!(mime, "application/json");
        assert!(serde_json::from_str::<Value>(&body).is_ok());
        for skill in SKILLS {
            let (mime, body) = read(&skill.entry_uri()).expect("playbook");
            assert_eq!(mime, "text/markdown");
            assert_eq!(body, skill.skill_md);
        }
    }

    #[test]
    fn resource_templates_cover_both_tiers() {
        let templates = resource_templates();
        let uris: Vec<_> = templates.iter().map(|t| t.uri_template.as_str()).collect();
        assert!(uris.contains(&"skill://{skill}/SKILL.md"));
        assert!(uris.contains(&"skill://{skill}/{+path}"));
    }

    #[test]
    fn referenced_files_finds_both_link_and_backticked_paths() {
        let doc = "see [a](references/a.md) and [b](./references/b.md#anchor), \
                   [ext](https://x.test/c.md), [img](d.png), and `references/e.md` \
                   plus `not a path.md` and `code.rs`";
        let links = referenced_files(doc);
        assert!(links.contains(&"references/a.md".to_string()));
        assert!(links.contains(&"./references/b.md".to_string()));
        assert!(
            links.contains(&"references/e.md".to_string()),
            "backticked path missed"
        );
        assert!(links.contains(&"https://x.test/c.md".to_string()));
        assert!(
            !links.iter().any(|l| l.contains(' ')),
            "prose captured: {links:?}"
        );
        assert!(!links.contains(&"code.rs".to_string()));
        assert!(!links.contains(&"d.png".to_string()));
    }

    #[test]
    fn frontmatter_parses_a_crlf_checkout() {
        // Windows git checks these out with CRLF. A parser that only knew LF
        // returned None for every field, so every trigger description was ""
        // there — a skill published that no request can match.
        const CRLF: &str = "---\r\nname: demo\r\ndescription: a windows one\r\n---\r\nbody\r\n";
        let fm = parse_frontmatter(CRLF).expect("parses");
        assert_eq!(fm["name"], "demo");
        assert_eq!(fm["description"], "a windows one");
        const CRLF_QUOTED: &str = "---\r\ndescription: \"quoted\"\r\n---\r\nbody\r\n";
        assert_eq!(
            parse_frontmatter(CRLF_QUOTED).unwrap()["description"],
            "quoted"
        );
    }

    #[test]
    fn frontmatter_is_verbatim_yaml_and_absent_is_none() {
        const DOC: &str = "---\nname: demo\ndescription: \"a quoted one\"\nlicense: MIT\n\
                           metadata:\n  version: \"2.1.0\"\n  tags: [a, b]\n---\nbody: not frontmatter\n";
        let fm = parse_frontmatter(DOC).expect("parses");
        assert_eq!(fm["name"], "demo");
        assert_eq!(fm["description"], "a quoted one");
        assert_eq!(fm["license"], "MIT");
        assert_eq!(fm["metadata"]["version"], "2.1.0");
        assert_eq!(fm["metadata"]["tags"], json!(["a", "b"]));
        assert!(!fm.contains_key("body"));
        assert!(parse_frontmatter("no frontmatter\n").is_none());
        assert!(parse_frontmatter("---\nunterminated: x\n").is_none());
        // A block that is not a mapping is not frontmatter.
        assert!(parse_frontmatter("---\n- just\n- a list\n---\n").is_none());
    }

    #[test]
    fn the_vendored_version_is_recorded() {
        // A catalog that reports 0.0.0 means the generated region was never
        // populated — the include_str! table would be empty too.
        assert_ne!(SKILLS_VERSION, "0.0.0", "skills were never vendored");
    }
}
