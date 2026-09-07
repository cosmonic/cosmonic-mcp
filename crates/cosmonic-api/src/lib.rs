//! Shared request/response types for the cosmonicd local API (v1).
//!
//! These types are the wire contract between the daemon and its clients
//! (Electron app, future CLI). See docs/ARCHITECTURE.md §3.3.

pub mod telemetry;
pub mod workload;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// `GET /v1/host`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    /// cosmonicd semver.
    pub version: String,
    /// Daemon lifecycle state.
    pub state: HostState,
    /// Seconds since the daemon started.
    pub uptime_secs: u64,
    /// k8s-style typed health conditions (Heartbeat / Ingress / ConsoleSync…)
    /// — the representable middle ground between "running" and "stopped" that
    /// the binary `state` cannot express (a lost ingress listener, an expired
    /// console session). Additive: empty from older daemons; `state` remains
    /// the computed roll-up (docs/CONTROL-PATTERNS.md #15).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<HostCondition>,
    /// `os/arch`, e.g. `darwin/arm64`.
    pub os: String,
    /// Friendly host name shown in the UI.
    pub friendly_name: String,

    // ---- Embedded wash-runtime host (ROADMAP 1.2). All optional so older
    // ---- clients (and a daemon whose host failed to report) stay compatible.
    /// Stable id of the embedded wash-runtime host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    /// Version of the embedded wash-runtime.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    /// Version-bump-invariant hash of the daemon's source tree, baked at
    /// build time (COSMONIC_DAEMON_SRC_HASH — scripts/daemon-src-hash.mjs).
    /// The app compares it to its bundled daemon's hash to decide whether an
    /// "update daemon" restart would actually change anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_src_hash: Option<String>,
    /// `true` when this daemon is a dev/source/test build (not produced by the
    /// release pipeline). Drives the "DEV BUILD" indicator in the app and is a
    /// deliberate policy signal so a test binary is never mistaken for a shipped
    /// release. Absent on older daemons (treated as non-dev).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_build: Option<bool>,
    /// Workload HTTP ingress address (`ip:port`) served by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_addr: Option<String>,
    /// Number of workloads currently scheduled on the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload_count: Option<u64>,
    /// Number of component instances across all workloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_count: Option<u64>,

    // ---- daemon process identity + on-demand metrics ----------------------
    // All optional (skip_serializing_if) so older clients are unaffected and
    // the endpoint degrades gracefully when a metric can't be collected.
    /// OS process id of the cosmonicd daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Path of the unix domain socket the local API is served on (the
    /// named-pipe path on Windows, once that transport lands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,
    /// Resident set size (RSS) of the cosmonicd process, in bytes. Collected
    /// on demand via sysinfo (this PID only) — no background polling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_rss_bytes: Option<u64>,
    /// CPU usage of the cosmonicd process as a percentage (can exceed 100 on
    /// multi-core). Collected on demand; the first call after startup may read
    /// 0 until sysinfo has two samples to diff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_cpu_percent: Option<f32>,
    /// System-wide CPU usage percentage, passed through from the host
    /// heartbeat (`system_cpu_usage`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_cpu_percent: Option<f32>,
    /// Total system memory in bytes (heartbeat `system_memory_total`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_memory_total: Option<u64>,
    /// Free system memory in bytes (heartbeat `system_memory_free`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_memory_free: Option<u64>,

    /// Messaging backend the host runs (`wasmcloud:messaging`). `"nats
    /// (host:port)"` by default — cross-workload messaging over a managed or
    /// existing core NATS server (docs/MESSAGING.md). `"in-memory"` when the
    /// host runs the per-workload `InMemoryMessaging` plugin (mode=memory, or
    /// NATS was requested but unavailable and it degraded). Optional/skip-if-none
    /// for older clients.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messaging: Option<String>,
    /// The `wasmcloud:nats` declaration the host was built with, as a label:
    /// default servers + `workloadConfig` policy + binding names (e.g.
    /// `nats://127.0.0.1:4222 (allow)`) — never a credential. Details on
    /// `GET /v1/nats` (daemon/docs/NATS.md). Absent on older daemons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nats: Option<String>,

    /// Count of workloads currently observed `Failed` (crash-looped or failed
    /// to start), across the spec-store + ephemeral-dev workloads the
    /// reconciler tracks (same population as `GET /v1/workloads`). Minimal
    /// error-state signal for tray clients (docs/NATIVE-TRAY.md) — `0` is a
    /// healthy fleet, `None` only if the reconciler's spec listing itself
    /// failed (disk error), matching `workload_count`'s optionality. Watch
    /// `GET /v1/events` for the per-workload `failed` `EventKind` for detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_workloads: Option<u64>,

    /// Host labels advertised in the heartbeat — the wasmCloud scheduling
    /// vocabulary (`hostcore.os`, `hostcore.arch`, `hostcore.osfamily`, plus any
    /// custom labels from `host.yaml`). Same role as a `wash` host's labels.
    /// Optional/skip-if-empty for older clients.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
}

/// One typed health condition on [`HostInfo::conditions`] — the k8s/crossplane
/// condition shape (type/status/reason/message), schema-ported from Control's
/// runtime-operator. `status` follows the k8s convention: "True" (healthy),
/// "False" (unhealthy), "Unknown".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostCondition {
    /// Condition name: "Heartbeat", "Ingress", "ConsoleSync", …
    #[serde(rename = "type")]
    pub condition_type: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

impl HostCondition {
    pub fn healthy(condition_type: &str) -> Self {
        Self {
            condition_type: condition_type.to_string(),
            status: "True".to_string(),
            reason: String::new(),
            message: String::new(),
        }
    }

    pub fn unhealthy(condition_type: &str, reason: &str, message: impl Into<String>) -> Self {
        Self {
            condition_type: condition_type.to_string(),
            status: "False".to_string(),
            reason: reason.to_string(),
            message: message.into(),
        }
    }
}

/// `GET /v1/inspect?image=<ref>` — extracted component metadata for the Inspect
/// screen. The component bytes are resolved offline-first from the local OCI
/// store, decoded with the `wit-parser`/`wasm-metadata` (wasm-tools) libraries,
/// and the result is cached on disk keyed by content digest
/// (`<state_dir>/inspect/<digest>.json`). Extraction is on-demand and never
/// blocks scheduling.
///
/// NOTE: fields stay snake_case (not camelCase like other transport DTOs)
/// because this struct is ALSO the on-disk inspect-cache format
/// (`read_cache`/`write_cache` in inspect.rs); renaming would change that format.
/// The only multi-word field is `component_digest`; the JS client reads it
/// snake_case at its single call site (screens_inspect.jsx).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectResponse {
    /// The pinned `image@sha256:…` reference inspected.
    pub reference: String,
    /// Manifest content digest (`sha256:…`).
    pub digest: String,
    /// The wasm component layer's content digest (`sha256:…`). This is the
    /// filename the component blob is stored under in the OCI layout, so the
    /// client can point at the exact file on disk.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub component_digest: String,
    /// Component size in bytes (the wasm layer).
    pub size: u64,
    /// The component's WIT world name, when it carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<String>,
    /// Imported interfaces, canonical `namespace:package/interface` names.
    #[serde(default)]
    pub imports: Vec<String>,
    /// Exported interfaces, canonical `namespace:package/interface` names.
    #[serde(default)]
    pub exports: Vec<String>,
    /// Source language, normalized to a short key (`rust`, `go`, `ts`, `js`,
    /// `python`, `java`, `c`) when detectable from the `producers` section.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The raw `producers` provenance string (e.g. `rustc 1.78 · wasm32-wasip2`),
    /// or None when the section was stripped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
}

/// `GET /v1/oci` element: one artifact in the local content-addressed OCI store
/// (the "Local Registry"). Built from the OCI layout index — cosign signature
/// artifacts are filtered out. Size is the component (wasm layer) size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalArtifact {
    /// Full ref name, e.g. `ghcr.io/acme/payment-router:0.4.1`.
    pub reference: String,
    /// Manifest content digest (`sha256:…`).
    pub digest: String,
    /// Component (wasm layer) size in bytes.
    pub size: u64,
    /// The component layer media type.
    pub media_type: String,
    /// Whether a cosign signature artifact for this digest is cached locally.
    pub signed: bool,
    /// User-locked: exempt from prune/delete (persisted in `<state>/oci/locks.yaml`).
    #[serde(default)]
    pub locked: bool,
    /// Referenced by a current workload spec or a retained revision — pruning is
    /// refused while in use.
    #[serde(default)]
    pub in_use: bool,
}

/// `POST /v1/oci/prune` request: the artifact manifest digests to delete.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciPruneRequest {
    pub digests: Vec<String>,
}

/// `POST /v1/oci/prune` response: what was actually freed vs. skipped.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciPruneResponse {
    pub reclaimed_bytes: u64,
    pub removed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<OciPruneSkip>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciPruneSkip {
    pub digest: String,
    /// `"locked"` or `"in-use"`.
    pub reason: String,
}

/// `GET /v1/hosts/discovered` element: a *best-effort* sighting of another
/// cosmonicd process on this machine (Host tab "other hosts").
///
/// Discovery scans the process table (sysinfo) for processes named
/// `cosmonicd`, excluding this one. It is intentionally cheap and lossy. The
/// HTTP ingress address and unix control socket path are recovered, best
/// effort, from the process's command line (e.g. `COSMONIC_HTTP_ADDR`) and —
/// when absent there (the common case, since they come from the inherited
/// environment) — from one `lsof -nP -p <pid>` call per discovered pid (the
/// TCP `LISTEN` line and the `*/cosmonicd.sock` unix entry). lsof is run on
/// request only, never on a background loop. macOS/Linux only: Windows has no
/// lsof, so there these fields stay absent (the app can still show the pid).
/// This is a discovery hint for the UI, not an authoritative inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredHost {
    /// OS process id.
    pub pid: u32,
    /// Process executable name (`cosmonicd`).
    pub name: String,
    /// Unix epoch seconds the process started, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Workload HTTP ingress address, if recoverable from the process args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_addr: Option<String>,
    /// Unix socket path, if recoverable from the process args.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostState {
    Starting,
    Running,
    Stopping,
}

/// `GET /v1/components` element: per-image deployment aggregation across all
/// workloads (Components tab — "how many workloads is this image deployed
/// in"). Derived live from the workload specs; carries no runtime state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDeployment {
    /// Image repo path with tag/digest stripped (the grouping key), e.g.
    /// `ghcr.io/acme/hello-world`.
    pub image: String,
    /// A representative component name using this image (first seen).
    pub name: String,
    /// `<namespace>/<name>` of every workload this image is deployed in.
    pub workloads: Vec<String>,
    /// Sum of `poolSize` (requested instances) across those deployments. Still
    /// desired, not actual, even now that wash-runtime v2.6.1 enforces
    /// `poolSize`: the runtime parks *up to* that many warm instances on
    /// demand and never pre-warms them, and on the HTTP path it pools P3
    /// components only (docs/SCALING.md §1).
    pub instances: u64,
}

/// Structured error body returned by every non-2xx response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

/// `GET /v1/workloads` element and the response to apply/start/stop: the
/// declarative spec plus the reconciler's observed status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkloadSummary {
    pub workload: workload::Workload,
    pub status: WorkloadStatus,
}

/// One entry in a workload's deployment-revision history (k8s-style). Returned
/// by `GET /v1/workloads/{ns}/{name}/revisions`, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionInfo {
    pub number: u64,
    pub created_at: String,
    pub active: bool,
    /// component/service name → pinned manifest digest (`sha256:…`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub image_digests: BTreeMap<String, String>,
    /// Component/service names captured by this revision (display summary).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<String>,
    pub spec_hash: String,
}

/// The revision history plus the effective retention window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionList {
    pub revisions: Vec<RevisionInfo>,
    pub retention: usize,
}

/// The global deployment-revision retention setting
/// (`GET`/`PUT /v1/config/revisions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetentionConfig {
    pub retention: usize,
}

/// Observed runtime status, kept by the reconciler — never written into the
/// spec YAML (ARCHITECTURE.md §4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadStatus {
    pub state: WorkloadState,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// Component name → pinned image digest (`sha256:…`) running right now.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub image_digests: BTreeMap<String, String>,
    /// RFC 3339 timestamp of the last state change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_transition: Option<String>,
    /// The deployment revision this status describes (the reconciler stamps
    /// it at deploy/rollback/restore). During an async apply or a blue-green
    /// cutover it is what tells a client whether `state=running` refers to
    /// the NEW spec or the still-serving previous one — Control's
    /// observedGeneration, ported (docs/CONTROL-PATTERNS.md #6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_revision: Option<u64>,
    /// Content hash of the spec at `observed_revision`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_hash: Option<String>,
    /// Consecutive crash-restarts attempted by the reconciler.
    #[serde(default)]
    pub restarts: u32,
    /// Total HTTP-ingress invocations observed for this workload since it was
    /// last (re)bound on the host. Counts only requests routed through the
    /// workload's HTTP ingress — messaging and other entrypoints are not
    /// observable and are not counted (see docs/SCALING.md). Optional so
    /// older clients and non-HTTP workloads (which never bind a counter) are
    /// unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocations_total: Option<u64>,
    /// HTTP-ingress invocations in the trailing minute (rolling rate derived
    /// from a per-second ring). Same scope/limits as `invocations_total`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocations_per_min: Option<u64>,
    /// Component name → the component's REAL WIT interfaces, extracted from its
    /// bytes at start (cached per digest). Lets the UI show actual capabilities
    /// instead of the declared `hostInterfaces`. Empty until extracted (the UI
    /// falls back to declared interfaces in the meantime).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub component_interfaces: BTreeMap<String, ComponentInterfaces>,
    /// What the reconciler is waiting for before it will pull/start this
    /// workload: `"credentials"` while it is **parked** at apply because a
    /// `secretFrom` ref / `configFrom` source is not registered on this host
    /// (docs/CREDENTIALS-DESIGN.md §7.10, accept-and-park). Absent otherwise.
    /// `state` stays `pending` while set, so older clients render it neutrally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_on: Option<String>,
    /// One row per credential the spec references (`secretFrom` refs,
    /// `configFrom` sources, `cosmonic:credentials` connections): its state,
    /// a message, and the one remediation sentence. Names and states only,
    /// never a value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<CredentialStatus>,
    /// The cosign verdict recorded for the serving start (`verified`,
    /// `unsigned_allowed`, `admit`, `override`): the weakest verdict across the
    /// workload's artifacts. Absent until a start ran the policy check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Which kind of credential a [`CredentialStatus`] row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CredentialKind {
    /// A `secretFrom: [{name}]` reference resolved through `/v1/secrets/refs`.
    #[default]
    SecretRef,
    /// A `configFrom: [{name}]` named config (`/v1/configs`).
    ConfigSource,
    /// A `cosmonic:credentials` connection binding (Layer 3, #481).
    Connection,
}

/// One credential a workload references, as the reconciler sees it
/// (docs/CREDENTIALS-DESIGN.md §7.10 "Status surface"). States for a
/// `secretRef`: `missing | registered | resolved | failed | ok | invalid |
/// insufficient`; for a `connection`: `missing | choose | consent-required |
/// connected | needs-reauth | revoked | missing-scope | disconnected | denied`.
/// Every field is a name, an env-var name, a scheme, a state or a sentence —
/// never a value.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    /// The ref / config / connection name.
    pub name: String,
    #[serde(default)]
    pub kind: CredentialKind,
    /// Environment variable the value is injected as (the registry's `env`,
    /// else the annotation's, else a suggestion derived from the name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<String>,
    /// Backend URI scheme of a registered ref (`keychain`, `env`, `op`, `aws-sm`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
    /// Connection kinds: the import label the manifest binds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<String>,
    /// The state vocabulary above.
    pub state: String,
    /// Short human-readable status line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    /// The one sentence a user (or an agent, verbatim) can act on. Empty for
    /// green states.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub remediation: String,
    /// From the `desktop.cosmonic.com/credentials` annotation (untrusted,
    /// capped, controls stripped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// https-only link where the value can be obtained (annotation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub obtain_url: Option<String>,
    /// The last `check_auth` probe verdict that named this credential.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_test: Option<CredentialTestResult>,
}

/// Identity a `check_auth` probe reported, reduced to the allowlisted fields.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
}

/// `POST /v1/workloads/{ns}/{name}/credentials/test` response and the
/// `lastTest`/`lastProbe` record: the daemon-run `check_auth` verdict
/// (docs/CREDENTIALS-DESIGN.md §7.10 "Test connection"). `status` is one of
/// `ok | missing | invalid | insufficient | unreachable | error | not_running
/// | not_mcp`; `message` is the verdict sentence rendered verbatim. Every
/// string passed through the daemon's scrubber (the workload's live secret
/// values and token shapes) and 512-char cap — never a token.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialTestResult {
    pub status: String,
    pub message: String,
    /// The one sentence the user (or an agent, verbatim) acts on. Always
    /// generated by the daemon (§7.9) — never copied from the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
    /// The server's own `remediation` text, if it sent one: scrubbed, capped,
    /// and **untrusted** — a hint about what capability or setting the
    /// upstream wants, shown attributed to the server. Never an instruction
    /// to the user or an agent, and never a substitute for `remediation`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<CredentialIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// The credential the verdict is about, when the probe could tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    /// RFC 3339.
    pub checked_at: String,
}

/// `POST /v1/workloads/{ns}/{name}/credentials/test` body.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialTestRequest {
    /// Scope the verdict sentence to one credential row (by name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// `GET /v1/workloads/{ns}/{name}/credentials` response: the status rows
/// merged with annotation metadata and, when the workload is running and
/// answers `GET /`, its live `credentials` discovery block.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadCredentials {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials: Vec<CredentialStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_probe: Option<CredentialTestResult>,
    /// Where the metadata came from: `annotation | live | spec-only`.
    pub source: String,
}

/// A component's real WIT world (canonical `namespace:package/interface`),
/// extracted from its bytes — the actual capabilities it imports/exports.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentInterfaces {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkloadState {
    /// Spec exists but the reconciler has not acted on it yet.
    #[default]
    Pending,
    Starting,
    Running,
    Completed,
    Stopping,
    /// Disabled (`enabled: false`) and not running.
    Stopped,
    /// Crashed or failed to start; `restarts` and `message` say why/how often.
    Failed,
}

/// `POST /v1/synthesize` request: an OCI ref (`ghcr.io/org/repo[:tag]`) or a
/// GitHub/GitLab repository URL (ARCHITECTURE.md §5.3).
///
/// NOTE: `use_credentials` is deliberately snake_case on the wire (no
/// `rename_all = "camelCase"`); the JS client already sends it snake_case
/// (data.js), so the format is frozen — do not "normalize" it to camelCase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesizeRequest {
    pub source: String,
    /// When true, the daemon may use the machine's local credentials (gh CLI,
    /// git credential helper, `GITHUB_TOKEN`) to reach a *private* source. Off
    /// by default: the first attempt is always anonymous. The UI re-sends with
    /// this set only after the user consents to a private-repo auth retry.
    #[serde(default)]
    pub use_credentials: bool,
}

/// `POST /v1/synthesize` response: a draft Workload the user reviews/edits
/// and then applies via `POST /v1/workloads`. The daemon never applies it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthesizeResponse {
    /// The first (best-ranked) draft, kept for back-compatibility with older
    /// callers that read a single `workload`. Equal to `workloads.first()`.
    /// `null` only for `source_kind: "wash-project"`, where the right flow is
    /// "open as project" (ROADMAP M4), not a workload.
    pub workload: Option<workload::Workload>,
    /// ALL parseable drafts from the source, ordered best-first (fewest
    /// unsupported interfaces, then HTTP ingress, then kind rank, then
    /// discovery order). A multi-document `WorkloadDeployment` manifest (or a
    /// repository with several manifests) yields one entry per workload; an
    /// OCI/wasm/gist single artifact yields exactly one. Each entry is a
    /// COMPLETE workload (all components plus any `service` sidecar). Empty
    /// only for `source_kind: "wash-project"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workloads: Vec<workload::Workload>,
    pub source_kind: SourceKind,
    /// Per-draft detail (source file, kind, content hash, duplicate flag, k8s
    /// labels) — one entry per `workloads[]` element, same order. The richer
    /// surface the multi-file wizard reads to render checkable per-file sections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub drafts: Vec<SynthDraft>,
    /// Human-readable findings the review UI should surface (inferred
    /// interfaces, unrecognized imports, deny-all egress, skipped manifests…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// Monorepo members discovered under an (unscoped) repo root: directories
    /// that hold a `.wash/config.yaml`. Surfaced so the Builder can render a
    /// pick-list — "Run" for members that publish an image, "Open in Builder"
    /// for source-only members — instead of dropping a multi-component repo as
    /// "nothing runnable". Empty for non-monorepo sources.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub buildable: Vec<BuildableMember>,
}

/// A monorepo member found during root discovery: a directory containing a
/// `.wash/config.yaml`. `image` is set when that config publishes an OCI ref
/// (the UI can offer "Run" straight away, synthesizing that subdirectory);
/// otherwise the member is source-only and the UI offers "Open in Builder".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildableMember {
    /// Repo-relative path to the member directory (the dir containing `.wash/`).
    pub path: String,
    /// Member display name (directory basename).
    pub name: String,
    /// Published OCI image ref, when the member's `.wash/config.yaml` names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// One synthesized draft with the provenance the multi-file wizard needs to
/// present it as a checkable section and reconcile it against what's deployed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SynthDraft {
    /// The complete draft workload (its `metadata.annotations` already carries
    /// `desktop.cosmonic.com/source-hash` = `content_hash`).
    pub workload: workload::Workload,
    /// Source file the draft came from (repo-relative), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Source manifest kind (`Workload` / `WorkloadDeployment` / `HTTPTrigger` /
    /// `oci` / `wasm` / `gist`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// sha256 of the draft (metadata sans annotations + spec) — identifies what
    /// the file deploys, so the wizard can flag new / unchanged / changed.
    pub content_hash: String,
    /// True when another draft in this batch describes the same component image
    /// set (default-unchecked in the wizard). `duplicate_of` names the kept one.
    #[serde(default)]
    pub duplicate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplicate_of: Option<String>,
    /// `hostInterface`s this host build cannot serve (may fail to start).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<String>,
    /// Per-draft notes (ingress derivation, dedup, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl SynthesizeResponse {
    /// Build a response from an ordered (best-first) list of drafts, setting
    /// the back-compat `workload` field to the first entry.
    pub fn from_workloads(
        workloads: Vec<workload::Workload>,
        source_kind: SourceKind,
        notes: Vec<String>,
    ) -> Self {
        Self {
            workload: workloads.first().cloned(),
            workloads,
            source_kind,
            drafts: Vec::new(),
            notes,
            buildable: Vec::new(),
        }
    }

    /// Build a response from rich per-file drafts (the multi-file path). Keeps
    /// `workload`/`workloads` in sync for back-compat.
    pub fn from_drafts(
        drafts: Vec<SynthDraft>,
        source_kind: SourceKind,
        notes: Vec<String>,
    ) -> Self {
        let workloads: Vec<workload::Workload> =
            drafts.iter().map(|d| d.workload.clone()).collect();
        Self {
            workload: workloads.first().cloned(),
            workloads,
            source_kind,
            drafts,
            notes,
            buildable: Vec::new(),
        }
    }

    /// A single-draft response (OCI/wasm/gist sources, and `wash-project` with
    /// `None`). Keeps `workload` and `workloads` in sync.
    pub fn single(
        workload: Option<workload::Workload>,
        source_kind: SourceKind,
        notes: Vec<String>,
    ) -> Self {
        let workloads = workload.clone().into_iter().collect();
        Self {
            workload,
            workloads,
            source_kind,
            drafts: Vec::new(),
            notes,
            buildable: Vec::new(),
        }
    }
}

/// Where the draft came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// Pulled an OCI component and inferred the spec from its WIT world.
    Oci,
    /// Imported/unwrapped a manifest found in the repository.
    Manifest,
    /// A `wash` project with nothing runnable published.
    WashProject,
}

/// `GET /v1/events` (SSE) payload — every reconciler transition emits one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// RFC 3339.
    pub ts: String,
    pub kind: EventKind,
    /// `<namespace>/<name>` of the workload concerned, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<String>,
    pub message: String,
    /// Monotonic per-boot sequence number. Carried as the SSE `id:` (prefixed
    /// with a boot nonce) so a reconnecting client resumes from the last event
    /// it saw instead of re-receiving the whole replay ring. `0` = emitted by
    /// a daemon predating the field.
    #[serde(default)]
    pub seq: u64,
    /// Aggregate count carried by kinds that summarize N occurrences into one
    /// event instead of emitting one-per-occurrence (currently only
    /// [`EventKind::Activity`] — the invocation-pulse sampler, see
    /// `activity_sampler.rs`). Absent (and omitted from the wire) for every
    /// other kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Applied,
    Started,
    Stopped,
    Removed,
    Restarting,
    Failed,
    Warning,
    /// Informational note (signature verified, mirror used, …) — additive,
    /// clients should render unknown kinds as plain notices.
    Info,
    /// One aggregated HTTP-ingress invocation pulse, emitted at most once per
    /// sample interval (10s; `cosmonicd::activity_sampler`) and only when
    /// invocations occurred since the previous sample — zero traffic means
    /// zero events, so an idle daemon never wakes an SSE client for this kind.
    /// `Event::count` carries the aggregate; `Event::message` is a
    /// human-readable summary (`"N invocation(s) in the last 10s"`). Never
    /// emitted per-workload (one event total per sample).
    Activity,
    /// A credential-state transition (docs/CREDENTIALS-DESIGN.md §8.2): a
    /// workload parked waiting for credentials / resumed, a secret ref
    /// rotated, a `check_auth` probe verdict. Additive — clients render
    /// unknown kinds as notices. Names only, never a value.
    Credential,
}

// ---- /v1/secrets/refs and /v1/configs (ARCHITECTURE.md §6, ROADMAP M3) -----

/// `GET /v1/secrets/refs` element. Deliberately *not* the full backend URI —
/// only its scheme — and never any secret material: the API must not leak
/// where or what a secret is beyond what the UI needs to render the ref.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretRefInfo {
    /// Ref name used by `secretFrom: [{name}]` in workload specs.
    pub name: String,
    /// Backend URI scheme (`keychain`, `env`, `op`, `aws-sm`).
    pub scheme: String,
    /// Environment variable the resolved value is injected as.
    pub env: String,
    /// Set when the ref points at a keychain service the daemon reserves for
    /// its own material (`"reserved-service"`): it can neither resolve nor be
    /// deleted through this API. Absent for every ordinary ref.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<String>,
}

/// `POST /v1/secrets/refs` body. Registers `name` → `uri` in the daemon's
/// secret-ref registry; a workload's `secretFrom: [{name}]` resolves the URI
/// at start and injects the value as the environment variable `env`.
///
/// `value` is **write-only**: when present (keychain URIs only), the secret
/// material is stored in the OS credential store at registration and is never
/// returned by any API afterwards.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSecretRefRequest {
    pub name: String,
    /// Backend URI: `keychain://<service>/<name>`, `env://VAR`,
    /// `op://vault/item/field`, `aws-sm://<region>/<secret-id>[#json-key]`.
    pub uri: String,
    /// Target environment variable name (`[A-Za-z_][A-Za-z0-9_]*`).
    pub env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Rotation: also restart the RUNNING workloads that reference this ref
    /// so they pick up the new value (env-injected values need a restart by
    /// construction). Default false, so existing scripts' behaviour is
    /// unchanged; workloads parked waiting for this ref start regardless.
    #[serde(default)]
    pub restart_dependents: bool,
}

// Manual Debug: the write-only `value` must never reach logs.
impl std::fmt::Debug for CreateSecretRefRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateSecretRefRequest")
            .field("name", &self.name)
            .field("uri", &self.uri)
            .field("env", &self.env)
            .field("value", &self.value.as_ref().map(|_| "<redacted>"))
            .field("restart_dependents", &self.restart_dependents)
            .finish()
    }
}

/// `POST /v1/secrets/refs` response: the registered ref plus the workloads
/// (`<namespace>/<name>`) this registration woke — `resumed` were parked
/// waiting for the ref and are now starting (always); `restarted` were
/// running and were restarted (only with `restartDependents: true`). Both
/// lists are always on the wire, empty included: the reply is the one
/// authoritative statement of what the daemon did, so a client never has to
/// guess from a missing key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSecretRefResponse {
    #[serde(flatten)]
    pub info: SecretRefInfo,
    #[serde(default)]
    pub resumed: Vec<String>,
    #[serde(default)]
    pub restarted: Vec<String>,
}

// ---- /v1/console (ARCHITECTURE.md §8, ROADMAP M6) ---------------------------

/// `GET /v1/console` response. Carries no token material, ever — tokens live
/// in the OS keychain and never cross the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleInfo {
    pub state: ConsoleState,
    /// Configured Console URL (also shown while disconnected, as a hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// RFC 3339 timestamp of the last successful device-flow completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    /// Host group this host advertises when registering with Control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_group: Option<String>,
    /// Signed-in account email (from the Console's `/v1/me`), when connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_email: Option<String>,
    /// Org this desktop is enrolled in (the personal org for now).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_org: Option<String>,
    /// Site id this desktop is enrolled as in the Console fleet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_id: Option<String>,
    /// Site name this desktop is enrolled as (e.g. `liams-macbook-pro`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    /// Human-readable detail (pending instructions, refresh failure reason).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleState {
    /// No Console configured (the default, fully functional air-gap state) or
    /// explicitly disconnected.
    #[default]
    Disconnected,
    /// Device-authorization flow in progress; the daemon is polling the token
    /// endpoint while the user approves in a browser.
    Pending,
    /// Tokens held in the keychain; refresh scheduled.
    Connected,
    /// Token refresh failed; reconnect required. Everything local keeps
    /// working.
    Expired,
}

/// `POST /v1/console/connect` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleConnectRequest {
    /// Console base URL, e.g. `https://console.corp.example`.
    pub url: String,
}

/// `POST /v1/console/connect` response: the RFC 8628 device-authorization
/// payload the UI shows (and uses to open the system browser — the daemon
/// never opens one). The daemon polls the token endpoint in the background;
/// progress is observable via `GET /v1/console` and `/v1/events`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleConnectResponse {
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    /// Seconds until the device code expires.
    pub expires_in: u64,
    /// Polling interval (seconds) the daemon honors.
    pub interval: u64,
}

// ---- /v1/catalog (ARCHITECTURE.md §5.2, ROADMAP 2.4) ------------------------

/// One curated catalog item ("Docker Hub style" browse screen). `image` is
/// present for runnable components — "Run" feeds it into `/v1/synthesize` —
/// and absent for display-only entries (templates, built-in capabilities).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogEntry {
    /// Stable identifier, unique within the catalog.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Publisher shown in the UI (e.g. "Cosmonic", "wasmCloud").
    pub publisher: String,
    pub category: CatalogCategory,
    pub description: String,
    /// OCI image ref for runnable entries; omitted for templates/capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Implementation/template languages, lowercase (`rust`, `go`, `tinygo`…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub langs: Vec<String>,
    /// Optional icon URL or bundled-asset name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogCategory {
    Template,
    Component,
    Capability,
}

/// `GET /v1/catalog` and `POST /v1/catalog/refresh` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogResponse {
    pub entries: Vec<CatalogEntry>,
    /// Where the entries came from: the catalog compiled into the daemon
    /// (`bundled`, always available offline) or a refreshed copy (`refreshed`,
    /// cached on disk across restarts).
    pub source: CatalogSource,
    /// RFC 3339 timestamp of the last successful refresh; only for
    /// `source: refreshed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatalogSource {
    Bundled,
    Refreshed,
}

// ---- /v1/policy (ARCHITECTURE.md §7, ROADMAP 5.1) ---------------------------
//
// These types deliberately use snake_case field names (no camelCase rename):
// they are also the on-disk format of `<state_dir>/policy.yaml` /
// `<state_dir>/mirrors.yaml`, whose documented shape is snake_case
// (`warn_unsigned: true`).

/// `GET /v1/policy` response and `PUT /v1/policy` body; persisted verbatim as
/// `<state_dir>/policy.yaml`. Rules are evaluated top-down against the full
/// repository path (`ghcr.io/org/repo`, no tag/digest); **first match wins**.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignaturePolicy {
    /// Strict signing (Settings → Security toggle). When `true`, **every** image
    /// must carry a cosign signature that verifies against a trusted key, or it
    /// is rejected — regardless of the matched rule's `require`, and even where
    /// no rule matches. When `false` (the default) signatures are advisory:
    /// verified when present, warned on per `warn_unsigned`, never blocking a
    /// start. Either way the daemon caches signatures at apply, so flipping this
    /// on does not make restore pay a per-workload registry round-trip.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strict: bool,
    #[serde(default)]
    pub registries: Vec<RegistryRule>,
}

/// One per-registry signature rule.
///
/// `require: signed` means **key-based cosign verification**: an ECDSA-P256
/// signature over the simple-signing payload, verified against one of the
/// configured `keys` (PEM strings or file paths). Keyless/Fulcio/Rekor chains
/// are explicitly out of scope — verification must work fully offline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryRule {
    /// Glob over the full repo path (`*` matches any run of characters,
    /// including `/`; `?` matches one), e.g. `ghcr.io/cosmonic/*`.
    pub pattern: String,
    #[serde(default)]
    pub require: SignatureRequirement,
    /// With `require: none`: emit a warning event when an admitted image has
    /// no verifiable signature.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub warn_unsigned: bool,
    /// Trusted cosign public keys: inline PEM (`-----BEGIN PUBLIC KEY-----…`)
    /// or absolute paths to PEM files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SignatureRequirement {
    /// Admit unsigned images (optionally warning, see `warn_unsigned`).
    #[default]
    None,
    /// Hard scheduling failure unless a configured key verifies the image
    /// (per-workload override: the `desktop.cosmonic.com/unsafe-allow-unsigned`
    /// annotation).
    Signed,
}

// ---- /v1/egress (wash-runtime v2.7.0 socket policy) --------------------------

/// `PUT /v1/egress` body and the `settings`/`active` halves of the
/// `GET /v1/egress` response; persisted verbatim as `<state_dir>/egress.yaml`
/// (snake_case, like `policy.yaml`). These are HOST-level knobs, applied when
/// the embedded runtime engine is built — a change takes effect on the next
/// daemon restart (`GET /v1/egress` reports both the stored and the active
/// values so a UI can show "restart required").
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EgressSettings {
    /// Raw `wasi:sockets` egress policy mode. `count` (the default) evaluates
    /// the policy, logs and meters what enforcement WOULD refuse, and allows
    /// it anyway — the upstream-default dry run. `enforce` refuses it, making
    /// the fail-closed `allowedHosts` contract cover raw sockets, not just
    /// `wasi:http`. A machine-layer floor can force `enforce`.
    #[serde(default)]
    pub socket_egress: SocketEgressMode,
    /// Host-level half of the two-key `host.wasmcloud.internal` door. A
    /// workload's `allowedHostLoopbackPorts` grant is inert unless this is
    /// also true (and vice versa — this alone grants nothing). Default false;
    /// a machine-layer floor can force it off, never on.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_host_loopback: bool,
    /// Trust roots for workload outbound HTTPS (`wasi:http` egress).
    /// `webpki` (default) is the compiled-in Mozilla bundle — upstream's
    /// default, kept so an upgrade never silently widens the trust boundary.
    #[serde(default)]
    pub trust_roots: TrustRootsMode,
    /// Extra PEM CA bundle paths trusted for workload outbound HTTPS, layered
    /// on top of `trust_roots` (corporate / private CAs). Bad bundles fail the
    /// daemon at startup rather than failing every request at runtime.
    /// Machine-layer bundles are merged in additively.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_ca_bundles: Vec<String>,
    /// Host-wide ceiling on concurrent guest connections (all workloads, all
    /// surfaces). Unset = unbounded. A machine-layer floor can lower it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<u32>,
    /// Per-workload ceiling on outbound pooled `wasi:http` connections
    /// (counting idle keep-alive). Unset = runtime default (128).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_http_per_workload: Option<u32>,
    /// Per-workload ceiling on outbound raw `wasi:sockets` connections.
    /// Unset = runtime default (256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_sockets_per_workload: Option<u32>,
    /// Per-workload ceiling on inbound published-port connections.
    /// Unset = runtime default (256).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inbound_sockets_per_workload: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SocketEgressMode {
    /// Evaluate, meter, allow anyway (dry run — the safe default).
    #[default]
    Count,
    /// Refuse what the policy refuses.
    Enforce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrustRootsMode {
    /// Compiled-in Mozilla roots only (upstream default).
    #[default]
    Webpki,
    /// Mozilla roots + the OS trust store.
    WebpkiAndNative,
    /// OS trust store only.
    Native,
}

/// `GET /v1/egress` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressStatus {
    /// What `<state_dir>/egress.yaml` holds now (with floors applied) — what
    /// the NEXT daemon start will use.
    pub settings: EgressSettings,
    /// What the running host was actually built with. Differs from `settings`
    /// after a PUT until the daemon restarts.
    pub active: EgressSettings,
    /// Live socket-policy decision counters since boot. `would_deny` counts
    /// what `enforce` WOULD have refused (non-zero = enforcing breaks
    /// someone); `denied` counts actual refusals under `enforce`.
    pub meters: Vec<EgressMeter>,
}

/// One socket-policy decision counter (see `EgressStatus::meters`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EgressMeter {
    /// Upstream's stable reason label: `not_permitted` |
    /// `bind_not_permitted` | `host_loopback_not_permitted` |
    /// `host_owned_port` | `blocked_range` | `no_capacity`.
    pub reason: String,
    pub denied: u64,
    pub would_deny: u64,
}

// ---- /v1/nats (daemon/docs/NATS.md; wash-runtime v2.8.0+ `wasmcloud:nats`) ----

/// The address a `wasmcloud:nats` binding that names no `servers` dials by
/// default: the NATS default port on the machine's own loopback.
pub const DEFAULT_NATS_SERVERS: &str = "nats://127.0.0.1:4222";

/// Who supplies a `wasmcloud:nats` binding's config — upstream's
/// `workloadConfig`. Desktop defaults to `allow` (a dev host: the manifest
/// carries its own grants, as `wash dev` does); connection keys are
/// host-owned regardless. `deny` is what `wash host` / Cosmonic Control run
/// and what an enterprise machine layer can force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatsWorkloadConfigMode {
    /// The manifest is authoritative for grants and behaviour.
    #[default]
    Allow,
    /// The declaration is the whole allowlist; a workload may only narrow it.
    Deny,
    /// Refuses nothing, logs what `deny` would refuse.
    Warn,
}

/// One named `wasmcloud:nats` binding (`(implements <name>)`), layered over
/// the top-level `config`/`secret_from`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct NatsBinding {
    /// Literal keys: grants, `servers`, path-valued credentials (`creds`,
    /// `tls-*`), client identity. Never a credential VALUE.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    /// Names of registered secret refs merged in last at boot; each ref's
    /// target key is the credential key it supplies (`token`, `password`,
    /// `nkey-seed`, `jwt`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_from: Vec<String>,
}

/// `<state_dir>/nats.yaml` — the operator's `wasmcloud:nats` declaration
/// (`GET`/`PUT /v1/nats`). Baked into the host at boot, so a PUT takes effect
/// on the next daemon restart; `GET` reports stored vs active. The enterprise
/// machine layer (`<machine_dir>/nats.yaml`, same keys) can only tighten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsSettings {
    /// Comma-separated NATS URLs a binding that names no `servers` dials.
    /// Default `nats://127.0.0.1:4222`; `COSMONIC_NATS_URL` overrides it for
    /// a boot. Empty = no default (every binding must name its own).
    #[serde(default = "default_nats_servers")]
    pub servers: String,
    #[serde(default)]
    pub workload_config: NatsWorkloadConfigMode,
    /// The base layer under every binding, named or not: grants
    /// (`subject-allow`, `stream-allow`, `bucket-allow` — ceilings under
    /// `deny`) and connection keys the operator sets host-wide.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    /// Secret refs merged into the base layer at boot (see [`NatsBinding`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_from: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, NatsBinding>,
    /// Extra keys claimed for the host under `deny` even though nothing here
    /// sets them (so an ungranted key resolves to nothing, not to the
    /// manifest's value).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_owned_keys: Vec<String>,
}

fn default_nats_servers() -> String {
    DEFAULT_NATS_SERVERS.to_string()
}

impl Default for NatsSettings {
    fn default() -> Self {
        Self {
            servers: default_nats_servers(),
            workload_config: NatsWorkloadConfigMode::default(),
            config: BTreeMap::new(),
            secret_from: Vec::new(),
            bindings: BTreeMap::new(),
            host_owned_keys: Vec::new(),
        }
    }
}

/// `GET /v1/nats`: stored vs active declaration plus boot-time diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NatsStatus {
    /// What `nats.yaml` holds now (machine floor applied; the env override is
    /// NOT folded in, so a client can edit and PUT this back without writing
    /// an environment value into the file) — what the NEXT daemon start will
    /// use, except that `env_override`, while set, replaces `servers`.
    pub settings: NatsSettings,
    /// What the running host was built with (env override + floor).
    pub active: NatsSettings,
    /// `COSMONIC_NATS_URL`, when set: it replaced `servers` for this boot and
    /// will again on the next one until unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_override: Option<String>,
    /// Secret refs the active declaration named that did not resolve at boot
    /// (ref names + backend error; never values). A binding missing its
    /// credential fails at bind with NATS's authorization error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_errors: Vec<String>,
}

/// `POST /v1/nats/test` body: dial a `wasmcloud:nats` connection from the
/// daemon — the process that really dials at bind — and report what answered.
/// A probe, not a save: nothing is persisted, and the running host is not
/// touched. Snake_case like [`NatsSettings`], so the Settings → Built-in plugins → NATS form can
/// post what it holds.
///
/// Two shapes, one route:
/// - **saved** — `name` alone: test `bindings.<name>` layered over the base
///   `config`/`secret_from` (+ the default `servers` bundle when the binding
///   names none), refs resolved exactly as boot resolves them. `""`/absent
///   is the unnamed default (base layer + default servers). The stored
///   declaration is used, with `COSMONIC_NATS_URL` applied the way a boot
///   applies it.
/// - **draft** — `servers`/`config`/`secret_from`: the layer the form holds
///   and has not saved. Any draft field present makes this a draft and `name`
///   is ignored. `servers` empty → the stored default servers.
///
/// A draft may carry a credential literally (`config.token`); it is used for
/// the one dial and dropped. `PUT /v1/nats` still refuses it — the store's
/// rule (values via `secret_from` only) is about what gets written to disk.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsTestRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Comma-separated NATS URLs, as [`NatsSettings::servers`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub servers: Option<String>,
    /// Literal keys, as [`NatsBinding::config`]. Grant and behaviour keys are
    /// accepted (the closed schema is enforced) and ignored by the probe.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    /// Secret refs merged in last, as [`NatsBinding::secret_from`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_from: Vec<String>,
}

// Manual Debug: a draft's `config` may carry a credential value.
impl std::fmt::Debug for NatsTestRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsTestRequest")
            .field("name", &self.name)
            .field("servers", &self.servers)
            .field("config", &self.config.keys().collect::<Vec<_>>())
            .field("secret_from", &self.secret_from)
            .finish()
    }
}

/// How the probe authenticated — the credential *kind* the resolved layer
/// selected, never its material. Mirrors the plugin's `NatsAuth::kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NatsTestAuth {
    /// No credential key in the layer: an anonymous dial.
    None,
    /// `token`.
    Token,
    /// `username` (`user`) + `password`.
    UserPassword,
    /// `nkey-seed` (`nkey`) alone: the server challenges, the seed signs.
    Nkey,
    /// `jwt` + `nkey-seed`: the JWT is presented, the seed signs the nonce.
    Jwt,
    /// `creds` (`creds-file`): a `.creds` file path the daemon reads itself.
    Creds,
}

impl NatsTestAuth {
    /// The wire spelling, for audit reasons and log lines.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Token => "token",
            Self::UserPassword => "user-password",
            Self::Nkey => "nkey",
            Self::Jwt => "jwt",
            Self::Creds => "creds",
        }
    }
}

/// Where a failed probe stopped, coarsest first — what the UI colours and what
/// the fix is: the declaration, the network, the credential, or the server's
/// JetStream setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NatsTestStage {
    /// Nothing was dialed: a secret ref did not resolve, the credential keys
    /// conflict, a creds file or nkey seed is unusable, or the address is one
    /// this build's client cannot dial (`ws://`).
    Resolve,
    /// The dial itself failed: refused, timed out, DNS, or TLS handshake.
    Connect,
    /// The server answered and refused the credential (or its absence).
    Auth,
    /// Reserved: a JetStream probe failure after a successful connect is
    /// reported as `ok: true` with `jetstream: false` and `jetstream_error`
    /// set, since the connection itself works.
    Jetstream,
}

impl NatsTestStage {
    /// The wire spelling, for audit reasons and log lines.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Resolve => "resolve",
            Self::Connect => "connect",
            Self::Auth => "auth",
            Self::Jetstream => "jetstream",
        }
    }
}

/// `POST /v1/nats/test` response — always HTTP 200 (a probe's failure *is*
/// its result; 400 is reserved for a malformed body). Never carries a
/// credential value: every sentence is scrubbed against the values the probe
/// resolved before it is returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatsTestResult {
    /// The server answered and accepted the connection.
    pub ok: bool,
    /// The comma-separated servers the probe dialed, after the default
    /// bundle and env override were applied — what a binding would dial.
    pub servers: String,
    /// The credential kind offered. On a `resolve` failure, the kind the keys
    /// that did resolve imply.
    pub auth: NatsTestAuth,
    /// A `tls://`/`wss://` address, `tls-first`, or (once connected) a server
    /// that requires TLS.
    pub tls: bool,
    /// On success: the dial alone — TCP, TLS, INFO/CONNECT and auth — in ms,
    /// excluding the secret-ref resolution that preceded it. On failure: wall
    /// time until the probe gave up (what the user waited for the verdict).
    pub latency_ms: u64,
    /// From the server's INFO, on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    /// On success: JetStream is enabled AND answered an account query for
    /// this connection (in the layer's `jetstream-domain`, when set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jetstream: Option<bool>,
    /// Why `jetstream` is `false` on an otherwise successful probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jetstream_error: Option<String>,
    /// On failure: where it stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<NatsTestStage>,
    /// On failure: one actionable sentence, safe to render verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A caveat on a success — e.g. the server did not require
    /// authentication, so the credential offered was never exercised (NATS
    /// ignores credentials when auth is off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ---- /v1/mirrors (ROADMAP 5.2) ----------------------------------------------

/// `PUT /v1/mirrors` body (the user's rules; the reply is [`MirrorStatus`]);
/// persisted as `<state_dir>/mirrors.yaml` after the machine floor is split
/// out. Applied at pull time to component *and* signature fetches; the longest
/// matching `from` prefix wins; specs keep the original ref.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MirrorConfig {
    #[serde(default)]
    pub mirrors: Vec<MirrorRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorRule {
    /// Ref prefix to replace, matched on `/`-`:`-`@` boundaries
    /// (e.g. `ghcr.io` or `ghcr.io/cosmonic`).
    pub from: String,
    /// Replacement prefix (e.g. `registry.corp.local:5000`).
    pub to: String,
    /// Allow plain-HTTP access to the mirror registry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub insecure: bool,
}

/// `GET /v1/mirrors` response (and the `PUT /v1/mirrors` reply): the
/// user-editable rules and the machine-layer floor **separately**, so clients
/// can render managed rules read-only and round-trip `mirrors` through a PUT
/// without absorbing the floor into the user's `mirrors.yaml`. `managed` rules
/// are authoritative: a ref any of them matches never consults `mirrors`, so a
/// more specific user rule can't route around the enterprise mirror.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MirrorStatus {
    /// The user's own rules — what a `PUT /v1/mirrors` body persists.
    #[serde(default)]
    pub mirrors: Vec<MirrorRule>,
    /// Enterprise machine-layer rules (`<machine_dir>/mirrors.yaml`): mandatory,
    /// locked, and not editable through the API.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed: Vec<MirrorRule>,
}

// ---- /v1/oci/import + /v1/oci/export (air-gap, ROADMAP 5.2) ------------------

/// `POST /v1/oci/import` body: `path` is an OCI image-layout directory or a
/// plain `.tar` of one, readable by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciImportRequest {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciImportResponse {
    pub artifacts: Vec<OciImportedArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciImportedArtifact {
    /// Original ref recorded in the layout (`org.opencontainers.image.ref.name`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Manifest digest (`sha256:…`), verified against the blob bytes.
    pub digest: String,
    pub kind: OciArtifactKind,
    /// Human-readable notes (signature verification outcome, policy hints…).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OciArtifactKind {
    Component,
    Signature,
}

/// `POST /v1/oci/export` body: write `ref` (and its cosign signature artifact,
/// when available) as an OCI image-layout at `path` (directory, or `.tar`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciExportRequest {
    #[serde(rename = "ref")]
    pub reference: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OciExportResponse {
    pub reference: String,
    pub digest: String,
    pub path: String,
    /// Whether the bundle includes the cosign signature artifact.
    pub signature_included: bool,
}

/// `GET /v1/configs` element and `POST /v1/configs` body: a named config
/// source referenced by `configFrom: [{name}]`. Plain (non-secret) string
/// maps, persisted as `<state_dir>/configs/<name>.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedConfig {
    pub name: String,
    #[serde(default)]
    pub config: BTreeMap<String, String>,
}

// ---- /v1/projects (project mode, ARCHITECTURE.md §5.4, ROADMAP M4) ----------

/// `GET /v1/projects` element: one registered local project. The registry
/// holds only `{id, path, name}` (`<state_dir>/projects.yaml`); everything
/// else is derived live from the project's `.wash/config.yaml` and the dev
/// session state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// DNS-label id derived from the directory name; path parameter for all
    /// per-project endpoints, and the dev workload's name in namespace `dev`.
    pub id: String,
    /// Display name (defaults to the directory name).
    pub name: String,
    /// Absolute project directory path. Registration never copies or deletes
    /// files; deregistration only forgets the entry.
    pub path: String,
    /// Parsed `.wash/config.yaml` summary; `None` when it is missing or
    /// invalid (see `configError`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ProjectConfigSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_error: Option<String>,
    /// Dev-loop state (ROADMAP 4.2).
    #[serde(default)]
    pub dev: DevInfo,
}

/// Summary of the project's `.wash/config.yaml` (wash v2 schema).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfigSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_command: Option<String>,
    /// Built component artifact path (project-relative or absolute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_path: Option<String>,
    /// Whether the config carries a `workload:` section (publish carries it
    /// over into the durable Workload draft, ROADMAP 4.3).
    #[serde(default)]
    pub has_workload_section: bool,
}

/// Dev session status, embedded in [`ProjectInfo`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevInfo {
    pub state: DevState,
    /// Human-readable detail (last failure reason, …).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_build: Option<BuildInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevState {
    #[default]
    Stopped,
    /// Build command running (initial or rebuild-on-change).
    Building,
    /// Ephemeral dev workload running on the local host (namespace `dev`).
    Running,
    /// Last build or start failed. A previous instance may still be serving
    /// (the dev loop keeps it running across failed rebuilds); `message`
    /// says which.
    Failed,
}

/// Outcome of the most recent dev/publish build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub ok: bool,
    pub duration_ms: u64,
    /// RFC 3339.
    pub finished_at: String,
}

/// `POST /v1/projects` body: register an existing project directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterProjectRequest {
    /// Absolute path of a directory containing `.wash/config.yaml`.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// `POST /v1/projects/new` body: scaffold a new project from a bundled
/// template, then register it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProjectRequest {
    /// Template id (see `GET /v1/projects/templates`): `rust-http`,
    /// `go-http`, `ts-http`.
    pub template: String,
    /// Directory to create (must not exist, or be empty).
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Response to `POST /v1/projects` and `POST /v1/projects/new`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectResponse {
    pub project: ProjectInfo,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// `GET /v1/projects/templates` element: a scaffolding template bundled in
/// the daemon (no network, no `wash` CLI required).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateInfo {
    pub id: String,
    /// Friendly name without the language ("HTTP", "NATS JetStream Consumer");
    /// the UI appends "· <Language>". Absent from daemons before the NATS
    /// starters landed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub language: String,
    pub description: String,
    /// Whether this template's build was verified on a development machine.
    /// Untested templates still scaffold, but their `build.command` may need
    /// toolchain work — `notes` lists the prerequisites.
    pub tested: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// `GET /v1/projects/{id}/dev/logs` response: the build/dev log ring buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevLogsResponse {
    pub lines: Vec<String>,
}

/// `POST /v1/workloads/validate` result: what an apply WOULD say, without
/// storing, pulling, or scheduling anything.
///
/// The only way to check a hand-authored spec used to be to apply it, which
/// stores it and starts pulling. An agent composing a manifest needs a
/// rehearsal that costs nothing and changes nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidateResponse {
    /// Whether an apply would be accepted.
    pub valid: bool,
    /// Schema and spec violations, in the order `Workload::validate` reports
    /// them. Empty when `valid`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// Things that would not block the apply but change what happens: an empty
    /// `allowedHosts` (deny-all), a secret reference this host does not have
    /// (accept-and-park), loopback ports that are inert because the door is
    /// shut.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// Secret references the spec names that are NOT registered here. Present
    /// even when `valid` — a spec naming a missing reference is accepted and
    /// parked, never rejected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_secret_refs: Vec<String>,
}

/// `POST /v1/projects/{id}/publish` body (ROADMAP 4.3): push to `ref` and return
/// a durable Workload manifest to review (review-then-apply, like
/// `/v1/synthesize` — nothing is scheduled here).
///
/// Publish pushes the project's *existing* built artifact; it only runs the
/// build command when there is no artifact yet, or `rebuild` is set.
///
/// Named `publish`, not `promote`: in ordinary deployment vocabulary "promote"
/// means making something live in the next environment, which is the one thing
/// this does NOT do. It publishes an artifact and schedules nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishRequest {
    /// Target OCI ref, e.g. `ghcr.io/acme/hello:0.1.0`.
    #[serde(rename = "ref")]
    pub reference: String,
    /// Allow plain-HTTP push (local registries only).
    #[serde(default)]
    pub insecure: bool,
    /// Force a fresh build even when a built artifact already exists. Default
    /// `false`: publish never touches the build path when the `.wasm` is present.
    #[serde(default)]
    pub rebuild: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResponse {
    /// The Workload manifest to review, with `image` pinned to the pushed
    /// digest. **Nothing is scheduled**: apply it to deploy.
    ///
    /// Named `workloadManifest`, not `workload`: the shorter name read as "a
    /// workload" — as though publishing had created one — which is the other
    /// half of the confusion the promote/publish rename addresses.
    pub workload_manifest: workload::Workload,
    /// Pinned `ref@sha256:…` that was pushed.
    pub image: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

// ---- /v1/logs (system log capture, docs/LOGS.md) ----------------------------
//
// The daemon captures every `tracing` event — its own logs (reconciler, api,
// host, oci…) AND component logs routed through the wasi:logging
// `TracingLogger` plugin — into a bounded in-memory ring, tagged with a
// [`LogSource`] so the UI can filter by host vs workload/component. See
// docs/LOGS.md for the capture pipeline and the source taxonomy.

/// Where a captured log record originated. Best-effort classification derived
/// from the event's fields/target (see docs/LOGS.md §source-taxonomy):
///
/// - [`LogSource::Component`] — a component log routed through the
///   `wasi:logging` `TracingLogger` plugin (carries `workload.component_id`,
///   `workload.name`, `workload.namespace`).
/// - [`LogSource::Workload`] — a daemon log carrying workload context (the
///   reconciler's `workload = "<ns>/<name>"` field) but no component identity.
/// - [`LogSource::Service`] — reserved for service-sidecar logs (a component
///   whose name is the reserved `service` sidecar). Currently sidecars log
///   through the same `wasi:logging` path as components.
/// - [`LogSource::Host`] — daemon-internal logs (reconciler/api/host/oci/…)
///   with no workload or component context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    Host,
    Workload,
    Service,
    Component,
}

impl LogSource {
    /// Parse a `source` query value (case-insensitive). Returns `None` for an
    /// unrecognized value so the handler can 400.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "host" => Some(Self::Host),
            "workload" => Some(Self::Workload),
            "service" => Some(Self::Service),
            "component" => Some(Self::Component),
            _ => None,
        }
    }
}

/// One captured log record (`GET /v1/logs` element and `/v1/logs/stream` SSE
/// frame data). Field names are camelCase on the wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRecord {
    /// RFC 3339 timestamp (millisecond precision, UTC).
    pub ts: String,
    pub level: LogLevel,
    pub source: LogSource,
    /// tracing target (module path or span name), e.g.
    /// `cosmonicd::reconciler` or `wash_runtime::plugin::wasi_logging`.
    pub target: String,
    /// `<namespace>/<name>` of the workload concerned, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<String>,
    /// Component id/name, for component/service-sourced records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Formatted human-readable message (the event's `message` field plus any
    /// remaining key=value fields appended).
    pub message: String,
    /// Remaining structured key/value fields captured from the event, after
    /// the message and the recognized identity fields are extracted.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, String>,
}

/// Log severity, ordered most→least severe to match tracing's levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    // Ordering matters: Error is the most severe. A `level=` query filters to
    // records at least as severe as the requested minimum, so Error <= Trace
    // numerically here (Error has the smallest discriminant). See
    // `LogLevel::at_least`.
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// Parse a `level` query value (case-insensitive). Returns `None` for an
    /// unrecognized value so the handler can 400.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "ERROR" => Some(Self::Error),
            "WARN" | "WARNING" => Some(Self::Warn),
            "INFO" => Some(Self::Info),
            "DEBUG" => Some(Self::Debug),
            "TRACE" => Some(Self::Trace),
            _ => None,
        }
    }

    /// True when `self` is at least as severe as `min` (i.e. would pass a
    /// `level=<min>` minimum-severity filter). Error passes every filter;
    /// Trace passes only `level=TRACE`.
    pub fn at_least(self, min: LogLevel) -> bool {
        // Smaller discriminant = more severe, so "at least as severe as min"
        // means our discriminant is <= min's.
        (self as u8) <= (min as u8)
    }
}

/// `GET /v1/logs` response: the filtered records plus the cap that was applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsResponse {
    /// Matching records, newest-first (most recent at index 0).
    pub records: Vec<LogRecord>,
    /// Whether the ring may hold older records than were returned (the result
    /// was truncated to `limit`).
    #[serde(default)]
    pub truncated: bool,
}

/// OTLP transport for the daemon's own export. The component `wasi-otel`
/// plugin is always gRPC (wash-runtime), so this only governs the host
/// process's exporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum OtlpProtocol {
    /// OTLP over gRPC (default; the OTel default port is 4317).
    #[default]
    Grpc,
    /// OTLP over HTTP/protobuf (the OTel default port is 4318).
    Http,
}

/// `GET /v1/logs/config` response and `PUT /v1/logs/config` body; persisted
/// verbatim as `<state_dir>/logs.yaml`. Snake_case on the wire AND on disk.
///
/// This is the OpenTelemetry (OTEL) configuration. Host logs always go to
/// stderr and the in-app capture ring; setting an endpoint and enabling export
/// ships the host's telemetry to one OTLP collector. `component_telemetry`
/// additionally exposes `wasi:otel` to workloads so they emit their own OTel to
/// the same endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LogConfig {
    /// Whether to export the daemon's own logs/traces via OTLP.
    #[serde(default)]
    pub otlp_enabled: bool,
    /// OTLP collector endpoint, e.g. `http://localhost:4317` (gRPC) or
    /// `http://localhost:4318` (HTTP). Export is a no-op unless this is set AND
    /// `otlp_enabled` is true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otlp_endpoint: Option<String>,
    /// Transport for the daemon's own export. Defaults to gRPC when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otlp_protocol: Option<OtlpProtocol>,
    /// `service.name` resource attribute on the daemon's own telemetry.
    /// Defaults to `cosmonicd` when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    /// Expose `wasi:otel` to components so workloads emit their own
    /// OpenTelemetry (gRPC) to the resolved endpoint. Applies on host restart.
    #[serde(default)]
    pub component_telemetry: bool,
}

/// Effective OTLP export status for the **current daemon run**. The endpoint
/// is resolved once at boot, so the persisted intent (`otlp_enabled`) and the
/// running exporter can disagree — this reports what is actually happening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OtlpStatus {
    /// Export is enabled and the exporter is running.
    Active,
    /// Export is enabled in `logs.yaml` but no endpoint was resolved at boot;
    /// the exporter is off for this run (the persisted intent is kept, so a
    /// restart with an endpoint configured picks it straight up).
    DisabledNoEndpoint,
    /// Export is enabled and an endpoint was resolved, but the exporter could
    /// not be started (see the daemon log for the build error).
    Error,
    /// Export is disabled.
    Off,
}

/// `GET`/`PUT /v1/logs/config` response: the persisted [`LogConfig`] plus the
/// effective OTLP status for this run. The config fields are flattened, so
/// existing clients that read `otlp_enabled` etc. keep working; `otlp_status`
/// is read-only (ignored if echoed back in a `PUT` body).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogConfigState {
    #[serde(flatten)]
    pub config: LogConfig,
    /// What the exporter is actually doing this run.
    pub otlp_status: OtlpStatus,
}

/// Which inference backend the host proxies `cosmonic:llm` to. All speak the
/// OpenAI Chat Completions wire schema; `ollama` is the batteries-included
/// default, `openai` is the generic escape hatch (llama-server / vLLM / llm-d /
/// a remote endpoint). The UI capability-gates pull/lifecycle off the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum InferenceBackend {
    #[default]
    Ollama,
    Openai,
}

/// `GET`/`PUT /v1/llm/config` — the Models / inference configuration, persisted
/// as `<state_dir>/llm.yaml`. The host serves `cosmonic:llm` (and the `wasi:llm`
/// alias) to components by proxying to `base_url` over the OpenAI-compatible
/// API. The daemon URL is editable without touching component bindings — the
/// `cosmonic:llm` abstraction the components see is stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Whether the host serves the built-in `cosmonic:llm` endpoint at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// The inference backend (`ollama` default, or a generic `openai` endpoint).
    #[serde(default)]
    pub backend: InferenceBackend,
    /// OpenAI-compatible base URL the host proxies to. For Ollama this is the
    /// daemon root (default `http://127.0.0.1:11434`); the host appends
    /// `/v1/chat/completions` etc. Empty → backend default.
    #[serde(default)]
    pub base_url: String,
    /// Default model alias resolved when a binding doesn't pin one.
    #[serde(default)]
    pub default_model: String,
    /// How long an idle model stays resident before unload (seconds). Maps to
    /// Ollama `keep_alive` / `OLLAMA_KEEP_ALIVE`. 0 → backend default.
    #[serde(default)]
    pub keep_alive_secs: u32,
    /// Unload models from memory after `keep_alive_secs`; cold-start on the next
    /// request. Capability-gated (Ollama supports it; a remote endpoint won't).
    #[serde(default = "default_true")]
    pub scale_to_zero: bool,
}

fn default_true() -> bool {
    true
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            backend: InferenceBackend::default(),
            base_url: String::new(),
            default_model: String::new(),
            keep_alive_secs: 0,
            scale_to_zero: true,
        }
    }
}

/// A model→workload binding: the host resolves `alias` (under `namespace`, on
/// `link_name`) to `model` for the given workload/component. Persisted in
/// `<state_dir>/llm-bindings.yaml`; this is the registry the host consults to
/// satisfy a component's `cosmonic:llm` import per workload.
///
/// NOTE: fields (e.g. `link_name`) are deliberately snake_case (no
/// `rename_all = "camelCase"`) — this is the wire format the JS client already
/// reads/sends (screens_models.jsx) AND the persisted `llm-bindings.yaml` disk
/// format; renaming would break both. Do not "normalize" it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmBinding {
    /// Stable id (assigned by the store on create; ignored on POST).
    #[serde(default)]
    pub id: String,
    /// The bound model name/alias, or `builtin` for the managed Cosmonic LLM.
    pub model: String,
    /// Target workload id.
    pub workload: String,
    /// Target component name; empty = all components in the workload.
    #[serde(default)]
    pub component: String,
    /// WIT namespace the component imports (`cosmonic:llm` | `wasi:llm`).
    pub namespace: String,
    /// Link name.
    #[serde(default)]
    pub link_name: String,
    /// The model alias the component requests (resolves to `model`).
    #[serde(default)]
    pub alias: String,
}

/// A locally-available model, as the Models UI table renders it. Sourced from
/// the backend's catalog (`/api/tags`) merged with what's currently loaded
/// (`/api/ps`). Fields are best-effort — absent ones are left empty/None.
///
/// NOTE: multi-word fields (`size_bytes`, `context_length`, `vram_bytes`) are
/// deliberately snake_case (no `rename_all = "camelCase"`); the JS client reads
/// them snake_case (screens_models.jsx), so the wire format is frozen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmModelInfo {
    /// Tagged name / alias, e.g. `llama3.2:3b`.
    pub name: String,
    /// Backend-agnostic reference, e.g. `ollama://library/llama3.2:3b`.
    pub reference: String,
    /// Model family, e.g. `llama`.
    #[serde(default)]
    pub family: String,
    /// Parameter size string, e.g. `3.2B`.
    #[serde(default)]
    pub params: String,
    /// Quantization, e.g. `Q4_K_M`.
    #[serde(default)]
    pub quant: String,
    /// On-disk size in bytes.
    #[serde(default)]
    pub size_bytes: u64,
    /// Context window length (tokens), when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u64>,
    /// Currently resident (loaded into memory / VRAM).
    #[serde(default)]
    pub loaded: bool,
    /// Resident size in bytes (VRAM/RAM) when loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vram_bytes: Option<u64>,
}

/// A chat message in the daemon's `/v1/llm/chat` DTO — the JSON projection of
/// the `cosmonic:llm` `message` (text content; multimodal parts arrive later).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
}

/// `POST /v1/llm/chat` (and `/v1/llm/chat/stream`) body — the host's
/// OpenAI-faithful chat request. `model` is an alias resolved by the host
/// (falls back to the configured default model). Mirrors the `cosmonic:llm`
/// `request` shape the WIT interface carries.
///
/// NOTE: `max_tokens` is deliberately snake_case (no `rename_all`) — it mirrors
/// the OpenAI wire schema field name, which the JS client reads/sends as-is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmChatRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub messages: Vec<LlmMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// `POST /v1/llm/chat` response — a unary completion.
///
/// NOTE: `finish_reason`/`prompt_tokens` are deliberately snake_case (no
/// `rename_all`) — they mirror the OpenAI wire schema field names, which the JS
/// client reads as-is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCompletion {
    pub model: String,
    pub content: String,
    #[serde(default)]
    pub finish_reason: String,
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

/// `GET /v1/host/config` response and `PUT /v1/host/config` body; persisted
/// verbatim as `<state_dir>/host.yaml`. Snake_case on the wire AND on disk.
///
/// `wasmcloud:messaging` is served by the built-in in-process host-wide broker
/// (docs/MESSAGING.md) — there is no backend to select. Legacy `messaging`/
/// `nats_url` fields in an older `host.yaml` are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HostConfig {
    /// Custom host labels (`key=value`) advertised in the heartbeat, on top of
    /// the built-in `hostcore.*` labels — the same knob as a `wash` host's
    /// `--label`. Applied at host build, so a change takes effect on the next
    /// daemon restart. Keys are validated (`hostcore.*` is reserved).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    /// Admin-pinned labels from the machine layer (`<machine_dir>/host.yaml`),
    /// laid down by an installer or MDM. **Read-only**: the daemon never writes
    /// this back into the user's `host.yaml` and a PUT carrying it is ignored,
    /// so the field is absent from disk and from a default install's response.
    /// The UI renders these as read-only chips beside the built-in `hostcore.*`
    /// set. A PUT naming one of these keys is rejected rather than silently
    /// losing to it at host build.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub managed_labels: BTreeMap<String, String>,
}

// ---- /v1/mcp/config (Model Context Protocol server, docs/MCP.md) -------------

/// `GET|PUT /v1/mcp/config`: whether the `cosmonicd mcp serve` MCP server is
/// enabled, and whether the daemon should auto-register it with the local Claude
/// client config so it "comes up with the daemon". Persisted to
/// `<state_dir>/mcp.yaml`. The MCP server is a stdio process the client spawns;
/// it connects back to this daemon over the unix socket. There is no secret
/// material here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    /// Master switch, **on by default** so the MCP server is available out of
    /// the box. When false, the daemon does not advertise or auto-register MCP
    /// (the `cosmonicd mcp serve` subcommand still works if invoked directly).
    /// An explicit `enabled: false` in `mcp.yaml` (the toggle always persists
    /// the field) is respected across restarts.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When enabled, ensure a `cosmonic` entry pointing at `cosmonicd mcp serve`
    /// exists in the local Claude Code (`~/.claude.json`) registration so the
    /// next Claude session can spawn it. Off by default — many users prefer to
    /// run `claude mcp add` themselves (the Settings screen shows the command).
    /// The `auto_register` alias keeps pre-camelCase `mcp.yaml` files (user and
    /// enterprise machine-layer) parsing.
    #[serde(default, alias = "auto_register")]
    pub auto_register: bool,
    /// Read-only: the absolute path to the `cosmonicd` binary the daemon resolved
    /// for the registration command (so the UI can show the exact `claude mcp
    /// add` line). Ignored on PUT.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_register: false,
            command: None,
        }
    }
}

// ---- /v1/registries (OCI registry credentials, ARCHITECTURE.md §7) ----------
//
// READ-ONLY listing never returns secrets: only the registry host, the
// username (when discoverable), and which source supplied the credential.

/// `GET /v1/registries` element: one configured registry credential, with the
/// source that won after dedup-by-host. Never carries a password/token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryInfo {
    /// Registry host, e.g. `ghcr.io`, `index.docker.io`, `localhost:5000`.
    pub registry: String,
    /// Username, when it can be determined without exposing the secret
    /// (decoded from a docker-config `auths` entry, or returned by a
    /// credential helper's `get`/`list`). Absent for helper-only entries we
    /// could not enumerate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Where the credential lives.
    pub source: RegistrySource,
    /// For `credential-helper` sources: the helper name
    /// (`docker-credential-<helper>`), e.g. `osxkeychain`, `wincred`, `pass`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub helper: Option<String>,
    /// Absolute path to a PEM CA bundle trusted when pulling from this
    /// registry (private/corporate CA), set via
    /// `POST /v1/registries/{host}/ca`. Layered on top of the built-in webpki
    /// roots for this registry's client only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_bundle: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistrySource {
    /// A base64 `user:pass` entry in `~/.docker/config.json` `auths`
    /// (or podman's `auth.json`).
    DockerConfig,
    /// A `credsStore`/`credHelpers` helper binary mints the token.
    CredentialHelper,
    /// Stored by cosmonicd in the OS keychain via `POST /v1/registries`.
    Keychain,
}

/// `GET /v1/registries` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryListResponse {
    pub registries: Vec<RegistryInfo>,
}

/// `POST /v1/registries` body: store a registry credential in the OS keychain.
/// `password` is **write-only** — it is stored at registration and never
/// returned by any API afterwards.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRegistryRequest {
    /// Registry host (`ghcr.io`, `localhost:5000`, …). No scheme, no path.
    pub registry: String,
    pub username: String,
    pub password: String,
}

/// `POST /v1/registries/{host}/ca` body: trust a PEM CA bundle for pulls from
/// that registry (its TLS chain is verified against webpki + this bundle).
/// Works with or without a stored credential — a private registry can need a
/// private CA and no auth. `DELETE /v1/registries/{host}/ca` clears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryCaRequest {
    /// Absolute path to a readable PEM CA bundle on this machine.
    pub path: String,
}

// Manual Debug: the write-only `password` must never reach logs.
impl std::fmt::Debug for CreateRegistryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateRegistryRequest")
            .field("registry", &self.registry)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// `POST /v1/registries/test` body: probe a registry's `/v2/` endpoint and, if
/// it challenges, exercise a credential against it — the "does this login
/// actually work?" check (issue #350). Nothing is stored and nothing is
/// changed; the credential in this body is used for exactly one probe.
///
/// Two shapes:
/// - `username`/`password` **present** — test those candidate credentials
///   without storing them (the Add-registry modal's *Test*).
/// - both **absent** — test the credential a pull would really resolve for
///   this host (our keychain entry → the docker chain → anonymous), which is
///   what the per-row *Test* in Settings → Registries asks.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryTestRequest {
    /// Registry host (`ghcr.io`, `localhost:5000`, …). No scheme, no path.
    pub registry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Write-only, like [`CreateRegistryRequest::password`]: sent to the
    /// registry being tested and then dropped. Never stored, logged, or
    /// returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

// Manual Debug: the write-only `password` must never reach logs.
impl std::fmt::Debug for RegistryTestRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryTestRequest")
            .field("registry", &self.registry)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What the probe concluded. Coarse on purpose — the sentence in
/// [`RegistryTestResponse::message`] carries the detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryTestResult {
    /// The registry answered and the credential (or anonymous access) was
    /// accepted.
    Ok,
    /// The registry answered and refused the credential — a wrong username /
    /// expired token, or none supplied where one is required.
    Unauthorized,
    /// The registry could not be reached at all (DNS, connection refused,
    /// timeout, TLS handshake).
    Unreachable,
    /// The registry answered with something we can't call a pass or a fail
    /// (not an OCI Distribution endpoint, rate limited, 5xx).
    Error,
}

/// Which credential the probe actually exercised. Never a secret — the
/// username at most, exactly as `GET /v1/registries` already reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryTestCredential {
    /// Supplied in the request body and not stored (testing before saving).
    Supplied,
    /// Our keychain entry for the host (`POST /v1/registries`).
    Keychain,
    /// The docker credential chain (`config.json` `auths`/helpers).
    DockerChain,
    /// The GitHub CLI token fallback — `ghcr.io` only.
    GithubCli,
    /// No credential was available; the probe was anonymous.
    None,
}

/// `POST /v1/registries/test` response. Never carries a password/token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryTestResponse {
    pub registry: String,
    pub result: RegistryTestResult,
    /// One sentence, safe to render verbatim.
    pub message: String,
    /// Which credential was exercised, and its username when there is one.
    pub credential: RegistryTestCredential,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// The registry served the probe without asking for a credential. With a
    /// credential present this means it was never exercised — an open registry
    /// can't tell you whether your login is good.
    pub anonymous: bool,
    /// Wall time of the probe, so a slow-but-working registry is visible.
    pub duration_ms: u64,
}

// ---- /v1/flags (feature flags, docs/FEATURE-FLAGS.md) -----------------------
//
// The daemon is the authority for runtime flag state: it layers compiled
// defaults < `<state_dir>/flags.yaml` < env (`COSMONIC_FLAG_<KEY>` /
// `COSMONIC_DRAFT`) and serves the resolved set here. The Electron app reads
// this to gate UI/tray surfaces, and the daemon itself gates draft routes.

/// Where a flag's resolved value came from (highest-precedence layer that set
/// it). Surfaced so the Labs UI can show "on via env" vs "on by you".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagSource {
    /// Compiled-in default (the stable baseline).
    Default,
    /// `<state_dir>/flags.yaml` (a persisted user override).
    File,
    /// `COSMONIC_FLAG_<KEY>` or `COSMONIC_DRAFT` environment variable.
    Env,
}

/// One feature flag's resolved state + metadata for the Labs UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagInfo {
    /// Stable identifier, e.g. `catalog`, `projects`, `settings.resources`.
    pub key: String,
    /// Resolved value after layering defaults < file < env.
    pub enabled: bool,
    /// Compiled default (the stable baseline; what ships with the flag off).
    pub default: bool,
    /// Whether this flag gates an in-progress (DRAFT) feature.
    pub draft: bool,
    /// Human label for the Labs settings panel.
    pub label: String,
    /// One-line description of what the flag gates.
    pub description: String,
    /// Which layer determined `enabled`.
    pub source: FlagSource,
}

/// `GET /v1/flags` response: the full resolved flag set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagsResponse {
    pub flags: Vec<FlagInfo>,
}

/// `PUT /v1/flags` body: set or clear a single persisted override in
/// `flags.yaml`. `enabled: null` clears the override (falling back to env /
/// default). An env override always wins over the file at read time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagOverrideRequest {
    pub key: String,
    /// `Some(true|false)` writes an override; `None` clears it.
    #[serde(default)]
    pub enabled: Option<bool>,
}

// ---- /v1/telemetry (crash & error reporting, docs/TELEMETRY.md) -------------
//
// Opt-in Sentry error reporting. Off by default; the user accepts a disclosure
// (or sets it in Settings → Privacy). The daemon layers env (COSMONIC_TELEMETRY)
// and the `--telemetry` flag over `<state_dir>/telemetry.yaml`. Reports are sent
// only when BOTH consent is on AND a Sentry DSN is configured.

/// `GET`/`PUT /v1/telemetry` body: the two independent consent axes.
///
/// `enabled` gates crash & error reporting (Sentry); `usage` gates product-usage
/// analytics (Amplitude, `usage.rs`). They are separate switches (Settings →
/// Privacy) so a user can opt into one without the other. `usage` is
/// `Option<bool>`: `None` (the field absent from an older `telemetry.yaml`)
/// means "follow the crash-reporting consent", preserving upgrade behavior when
/// the two shared one gate; `Some(_)` is an explicit choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<bool>,
}

/// One product-telemetry event on its way in from the Electron app
/// (`POST /v1/telemetry/events`). The daemon is the ONE outbound door, so the
/// renderer and main process hand events over rather than sending them.
///
/// Nothing here is trusted: the event name is checked against the daemon's
/// catalog, unknown properties are dropped, and every value goes through the
/// scrubber before it can be enqueued (docs/telemetry/README.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    /// A catalog event name (`component_deployed`, …). Anything else is dropped.
    pub event: String,
    /// The property bag. Non-object values are ignored.
    #[serde(default)]
    pub properties: serde_json::Value,
    /// Which process produced it: `renderer` | `main` | `daemon` | `mcp`.
    /// Unrecognized values are normalized to `daemon`.
    #[serde(default)]
    pub source_process: Option<String>,
}

/// `POST /v1/telemetry/events` body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryEventBatch {
    #[serde(default)]
    pub events: Vec<TelemetryEvent>,
}

/// `POST /v1/telemetry/events` response: how many of the submitted events were
/// accepted. A rejection is not an error — it means the catalog or the scrubber
/// did its job — so the app never retries on this number.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryEventAck {
    pub accepted: usize,
    pub rejected: usize,
}

// ---- cosmonic:notify (docs/NOTIFY.md) ------------------------------------

/// `GET`/`PUT /v1/notify/config` — notification policy, persisted as
/// `<state_dir>/notify.yaml`.
///
/// NOTE: snake_case on the wire. This DTO **is** the on-disk YAML (the
/// `LlmConfig` precedent), and an operator editing `notify.yaml` by hand — or
/// shipping one through the machine layer — should not have to think in
/// camelCase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyConfig {
    /// Master switch. Off means every send returns `unavailable`, whatever the
    /// platform can do.
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub limits: NotifyLimits,
    /// Per-workload policy, keyed by `<namespace>/<name>`.
    ///
    /// Workload-scoped rather than component-scoped because wash-runtime
    /// exposes no stable component *name* — only a per-instance UUID — and a
    /// key built from that would change on every restart, silently
    /// un-revoking what the user revoked (see `notify::types::Caller`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub components: BTreeMap<String, NotifyComponentPolicy>,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            limits: NotifyLimits::default(),
            components: BTreeMap::new(),
        }
    }
}

/// Rate-limit shape. Designed now, **not enforced in v1** — the service counts
/// breaches and logs them so the numbers are real before anything is rejected
/// (docs/NOTIFY.md §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyLimits {
    /// Ceiling across every component on this host.
    #[serde(default = "default_global_per_hour")]
    pub global_per_hour: u32,
    /// Burst any single component may fire back-to-back.
    #[serde(default = "default_burst")]
    pub default_burst: u32,
    /// Sustained per-component rate.
    #[serde(default = "default_per_hour")]
    pub default_per_hour: u32,
}

fn default_global_per_hour() -> u32 {
    60
}
fn default_burst() -> u32 {
    3
}
fn default_per_hour() -> u32 {
    12
}

impl Default for NotifyLimits {
    fn default() -> Self {
        Self {
            global_per_hour: default_global_per_hour(),
            default_burst: default_burst(),
            default_per_hour: default_per_hour(),
        }
    }
}

/// One component's notification policy. Absent = allowed with the defaults.
///
/// `Default` is written out rather than derived on purpose: `allowed` must
/// default to **true**, and `#[serde(default = "default_true")]` governs only
/// deserialization — a derived `Default` would make `allowed` false and
/// silently revoke every component that has no explicit entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotifyComponentPolicy {
    /// `false` = the user revoked notifications for this component in
    /// Settings → Notifications. Sends return `notify-error::denied`.
    #[serde(default = "default_true")]
    pub allowed: bool,
    /// Optional per-component override of `limits.default_per_hour`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_hour: Option<u32>,
}

impl Default for NotifyComponentPolicy {
    fn default() -> Self {
        Self {
            allowed: true,
            per_hour: None,
        }
    }
}

/// What the platform can actually do right now, as reported to the UI.
/// Mirrors the WIT `platform-capabilities` record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyCapabilities {
    /// False when nothing can be displayed at all — headless Linux session,
    /// macOS authorization refused, or a daemon with no bundle identity.
    pub available: bool,
    /// Stable backend key: `os`, `mock`, or `none`.
    pub backend: String,
    /// Why `available` is false, in one human sentence. Empty when available.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    pub actions: bool,
    pub inline_reply: bool,
    pub urgency: bool,
    pub persistent: bool,
    pub max_actions: u8,
}

/// One row of Settings → Notifications: a workload that has asked to notify,
/// with its live counters. Sorted most-recent-first by the API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyComponentStatus {
    /// Policy key — `<namespace>/<name>`.
    pub key: String,
    pub namespace: String,
    pub name: String,
    /// Resolved from config; `false` once the user revokes it.
    pub allowed: bool,
    /// Notifications accepted since the daemon started.
    pub sent: u64,
    /// Sends rejected because the user revoked this component.
    pub denied: u64,
    /// Sends that breached the rate limit. v1 logs these and lets them through.
    pub throttled: u64,
    /// Responses still queued for this component's next `events.pull`.
    pub queued: u32,
    /// RFC 3339 timestamp of the last accepted send, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sent: Option<String>,
}

/// `POST /v1/notify/simulate` body — deliver a simulated user response so the
/// whole round trip can be exercised where the real notification path is
/// unavailable (a dev daemon, a headless CI runner). Only honoured when the
/// daemon was started with `COSMONIC_NOTIFY_BACKEND=mock`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifySimulateRequest {
    pub id: u64,
    /// `activated` | `action` | `reply` | `dismissed` | `expired`.
    pub kind: String,
    /// The action or input id, for `action` and `reply`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_id: Option<String>,
    /// The typed text, for `reply`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// `GET /v1/notify` — everything Settings → Notifications needs in one call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyStatus {
    pub config: NotifyConfig,
    pub capabilities: NotifyCapabilities,
    pub components: Vec<NotifyComponentStatus>,
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    // Locks the HostInfo wire contract to camelCase (matches the JS client, which
    // reads e.g. friendlyName / uptimeSecs / processRssBytes). A snake_case
    // regression here was the original "no host metrics" bug class.
    #[test]
    fn host_info_serializes_camel_case() {
        let info = HostInfo {
            version: "0.3.0".into(),
            state: HostState::Running,
            uptime_secs: 12,
            conditions: vec![HostCondition::healthy("Heartbeat")],
            os: "darwin/arm64".into(),
            friendly_name: "cosmonic-desktop".into(),
            host_id: Some("h-1".into()),
            runtime_version: Some("v2.5.0".into()),
            daemon_src_hash: Some("abcd1234".into()),
            dev_build: Some(true),
            http_addr: Some("127.0.0.1:8200".into()),
            workload_count: Some(2),
            component_count: Some(3),
            pid: Some(42),
            socket_path: Some("/run/cosmonicd.sock".into()),
            process_rss_bytes: Some(1024),
            process_cpu_percent: Some(1.5),
            system_cpu_percent: Some(9.0),
            system_memory_total: Some(8 << 30),
            system_memory_free: Some(4 << 30),
            messaging: Some("in-memory".into()),
            nats: Some("nats://127.0.0.1:4222 (allow)".into()),
            failed_workloads: Some(1),
            labels: BTreeMap::new(),
        };
        let v: serde_json::Value = serde_json::to_value(&info).unwrap();
        for key in [
            "friendlyName",
            "uptimeSecs",
            "hostId",
            "runtimeVersion",
            "httpAddr",
            "workloadCount",
            "componentCount",
            "socketPath",
            "processRssBytes",
            "processCpuPercent",
            "systemCpuPercent",
            "systemMemoryTotal",
            "devBuild",
            "failedWorkloads",
        ] {
            assert!(
                v.get(key).is_some(),
                "HostInfo must emit `{key}` (camelCase)"
            );
        }
        // and NOT the snake_case forms
        for key in ["friendly_name", "uptime_secs", "process_rss_bytes"] {
            assert!(
                v.get(key).is_none(),
                "HostInfo must NOT emit snake_case `{key}`"
            );
        }
    }

    // Locks EventKind's existing lowercase convention for the new `Activity`
    // kind, and that `Event::count` rides along camelCase-free (Event is NOT
    // `rename_all = "camelCase"` — only `kind`/`message`/`workload`/`seq`/
    // `count` are already all-lowercase single words, so this has never needed
    // renaming; verify count doesn't silently regress that).
    #[test]
    fn event_activity_kind_serializes_lowercase_with_count() {
        let event = Event {
            ts: "2026-07-21T00:00:00Z".into(),
            kind: EventKind::Activity,
            workload: None,
            message: "3 invocation(s) in the last 10s".into(),
            seq: 42,
            count: Some(3),
        };
        let v: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert_eq!(v.get("kind").and_then(|k| k.as_str()), Some("activity"));
        assert_eq!(v.get("count").and_then(|c| c.as_u64()), Some(3));
        assert!(
            v.get("workload").is_none(),
            "workload must be omitted when None"
        );
    }

    // Existing kinds must keep serializing with `count` entirely absent from
    // the wire (not `null`) — old clients that don't know the field must see
    // exactly the same JSON shape as before this change.
    #[test]
    fn event_without_count_omits_the_field() {
        let event = Event {
            ts: "2026-07-21T00:00:00Z".into(),
            kind: EventKind::Started,
            workload: Some("default/hello".into()),
            message: "workload started".into(),
            seq: 1,
            count: None,
        };
        let v: serde_json::Value = serde_json::to_value(&event).unwrap();
        assert!(v.get("count").is_none(), "count must be omitted when None");
    }

    // A DERIVED Default here would make `allowed` false, silently revoking
    // every component that has no explicit policy row — the permission model
    // inverted by an attribute that only governs deserialization.
    #[test]
    fn absent_notification_policy_means_allowed() {
        assert!(
            NotifyComponentPolicy::default().allowed,
            "a component with no policy row must be ALLOWED, not revoked"
        );
        let from_empty: NotifyComponentPolicy = serde_json::from_str("{}").unwrap();
        assert!(
            from_empty.allowed,
            "an omitted `allowed` must also mean allowed"
        );
        let revoked: NotifyComponentPolicy = serde_json::from_str(r#"{"allowed":false}"#).unwrap();
        assert!(!revoked.allowed);
    }

    #[test]
    fn notify_config_defaults_are_permissive_and_bounded() {
        let c = NotifyConfig::default();
        assert!(
            c.enabled,
            "notifications ship on; the per-component control is the gate"
        );
        assert!(c.components.is_empty());
        assert!(c.limits.default_per_hour > 0 && c.limits.global_per_hour > 0);
    }
}
