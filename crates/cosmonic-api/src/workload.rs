//! `runtime.wasmcloud.dev/v1alpha1` Workload spec — the subset Cosmonic
//! Desktop schedules (ARCHITECTURE.md §4). Field names and semantics match the
//! wasmCloud runtime-operator CRD so specs are portable to/from a Cosmonic
//! Control cluster. Desktop-local state (e.g. enabled) lives in annotations,
//! never in new fields.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "runtime.wasmcloud.dev/v1alpha1";
pub const KIND_WORKLOAD: &str = "Workload";

/// Annotation marking whether the reconciler should keep this workload running.
pub const ANNOTATION_ENABLED: &str = "desktop.cosmonic.com/enabled";

/// Per-workload signature-policy override (ARCHITECTURE.md §7, ROADMAP 5.1):
/// `"true"` admits images that a `require: signed` policy rule would reject.
/// Recorded in the spec on purpose — the override is explicit and auditable,
/// and every use is flagged with a warning event by the reconciler.
pub const ANNOTATION_UNSAFE_ALLOW_UNSIGNED: &str = "desktop.cosmonic.com/unsafe-allow-unsigned";

/// Provenance content hash stamped by synthesis: sha256 of the draft (metadata
/// sans annotations + spec). Lets the multi-file wizard reconcile a directory's
/// files against what's deployed (new / unchanged / changed). Excluded from the
/// revision spec hash (annotations aren't hashed there), so it never churns
/// revisions.
pub const ANNOTATION_SOURCE_HASH: &str = "desktop.cosmonic.com/source-hash";

/// Annotation recording which surface applied a workload (best-effort, set at
/// apply time). Surfaced in `GET /v1/workloads` and aggregated by usage
/// analytics as the deployment-origin mix. NOT part of the revision spec hash
/// (annotations aren't hashed), so it never churns revisions. The value is one
/// of the `SOURCE_*` constants below; anything else is treated as unknown.
pub const ANNOTATION_SOURCE: &str = "desktop.cosmonic.com/source";
/// Applied through the Electron desktop UI.
pub const SOURCE_UI: &str = "ui";
/// Applied through the MCP server (`cosmonicd mcp serve`) — an AI agent/tool.
pub const SOURCE_MCP: &str = "mcp";
/// Applied by a direct/unlabeled API client over the socket (scripts, ad-hoc
/// `curl`). The first-party `cosmonic` CLI tags itself `SOURCE_CLI` instead.
pub const SOURCE_API: &str = "api";
/// Pushed by a Cosmonic Console over the washlet control channel.
pub const SOURCE_WASHLET: &str = "washlet";
/// Applied by the first-party `cosmonic` developer CLI (a human at a terminal),
/// distinct from `SOURCE_MCP` (an AI agent) so the deployment-origin mix can
/// tell them apart.
pub const SOURCE_CLI: &str = "cli";

/// Layer 2 credential metadata (docs/CREDENTIALS-DESIGN.md §6.2): a JSON
/// array string of `{kind, ref, env, description, obtainUrl, scopes,
/// validate}` entries describing the secrets a server needs, so the UI and
/// agents can prompt for them *before* the workload runs. **Never
/// authorization**: it cannot add a `secretFrom`, name a connection or widen
/// anything — only refs the spec references are prompted for. Untrusted
/// input: parsed leniently and capped by the daemon.
pub const ANNOTATION_CREDENTIALS: &str = "desktop.cosmonic.com/credentials";

/// The recognized `ANNOTATION_SOURCE` values.
pub const KNOWN_SOURCES: &[&str] = &[
    SOURCE_UI,
    SOURCE_MCP,
    SOURCE_API,
    SOURCE_WASHLET,
    SOURCE_CLI,
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workload {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub spec: WorkloadSpec,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub name: String,
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// k8s-style labels (e.g. `app.kubernetes.io/name`, `/version`). Carried
    /// from source manifests for display (the Workloads grid surfaces the
    /// app name + version); not used for scheduling or the revision spec hash.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

fn default_namespace() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_interfaces: Vec<HostInterface>,
    pub components: Vec<Component>,
    /// Optional long-running **service sidecar** (the wasmCloud v2 Workload's
    /// `service`): a component exporting `wasi:cli/run` that owns localhost
    /// ports (e.g. the service-tcp template). Drives the UI Services tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<WorkloadService>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volumes: Vec<Volume>,

    // ---- Control-side scheduling fields, preserved for spec portability ----
    // The runtime.wasmcloud.dev/v1alpha1 schema is shared with Cosmonic
    // Control (runtime-operator) ON PURPOSE. This single-host daemon does not
    // schedule by selector/id/environment, but it must ROUND-TRIP these
    // fields: the store re-serializes specs through this struct, so anything
    // missing here is silently stripped from a Control-authored YAML on apply
    // (docs/CONTROL-PATTERNS.md #2). Ignored fields are surfaced as an Info
    // event at apply, never enforced.
    /// Control: label selector choosing which hosts may run this workload.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub host_selector: BTreeMap<String, String>,
    /// Control: pin to one host by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    /// Control: deployment environment used for trigger/host matching.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// Control: Kubernetes integration block (e.g. `service.name`). Opaque —
    /// preserved byte-for-byte so future Control sub-fields survive too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubernetes: Option<serde_json::Value>,
}

/// A service sidecar — matches the runtime-operator CRD / `wash_runtime`
/// `types::Service` shape. Like a component it is pulled by OCI ref and pinned
/// at apply time, but it is *long-running and restartable* (`max_restarts`)
/// rather than pooled/invoked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadService {
    /// Display name (defaults to `"service"` when absent). Not part of the
    /// runtime `types::Service`, but useful for the UI Services tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// OCI reference; pinned to `image@sha256:…` by the daemon at apply time.
    pub image: String,
    /// Control: reference to a k8s pull-credential Secret (`{name}`).
    /// Preserved for round-trip portability; Desktop resolves registry
    /// credentials through its own registry store instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_secret: Option<LocalObjectReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_policy: Option<String>,
    /// Restart cap for the service supervisor (runtime
    /// `types::Service::max_restarts`). Absent = use the runtime default (0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_restarts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_resources: Option<LocalResources>,
}

/// k8s `corev1.LocalObjectReference` — a by-name reference to an object in
/// the same namespace. Carried for Control portability (imagePullSecret).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalObjectReference {
    pub name: String,
}

/// One granted host capability interface (maps 1:1 to
/// `wash_runtime::wit::WitInterface`) plus its config/configFrom/secretFrom
/// layers; `name` is the instance key for multiplexing plugins.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInterface {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub namespace: String,
    pub package: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_from: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_from: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub name: String,
    /// OCI reference; pinned to `image@sha256:…` by the daemon at apply time.
    pub image: String,
    /// Control: k8s pull-credential Secret reference — see
    /// [`WorkloadService::image_pull_secret`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_secret: Option<LocalObjectReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_pull_policy: Option<String>,
    /// Warm instances parked between calls (runtime
    /// `types::Component::pool_size`, i32 — clamped on conversion). Serde
    /// default 1; `0` means a fresh store per call.
    ///
    /// **Enforced as of wash-runtime v2.6.1** (`engine::InstancePolicy`) — but
    /// only on the pooled paths: HTTP ingress pools **P3 components only**
    /// (`wasi:http/handler@0.3.0`), while a P2 `incoming-handler` component
    /// accepts this field and ignores it. Linked component-to-component calls
    /// pool either way. See docs/SCALING.md §1 before surfacing this as a
    /// live scaling control — for most components today it is still inert.
    #[serde(default = "default_pool_size")]
    pub pool_size: u32,
    /// Calls one warm instance may serve before being retired (runtime
    /// `types::Component::max_invocations`). Default 0 = no cap. Enforced
    /// alongside `pool_size` on the same paths (docs/SCALING.md §1); it bounds
    /// how stale a warm instance's frozen context can get.
    ///
    /// Not an invocation *counter*: Desktop's invocation counts come from its
    /// own HTTP-ingress router counter, not the runtime.
    #[serde(default)]
    pub max_invocations: u32,
    /// Calls one warm instance may serve *concurrently* (runtime
    /// `types::Component::max_concurrency`, new in wash-runtime v2.7.0).
    /// Default 0 = one at a time, exactly like an unpooled instance. Raising
    /// it lets an instance overlap calls while awaiting I/O — only safe for a
    /// guest that yields rather than blocks (a guest driving its own executor
    /// with `block_on` must stay at one). Same wire field as Control's CRD.
    #[serde(default)]
    pub max_concurrency: u32,
    /// How long (seconds) the warm-instance pool watches its own peak
    /// concurrency before retiring the instances that peak did not need
    /// (runtime `types::Component::reclaim_window_seconds`, new in
    /// wash-runtime v2.8.0). Default 0 = never reclaim for idleness: the pool
    /// grows to `poolSize` under load and keeps what its busiest moment
    /// needed until the workload stops — the pre-v2.8.0 behavior. Same wire
    /// field as the washlet v2 protocol; Control's CRD does not carry it yet.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub reclaim_window_seconds: u32,
    /// Warm instances a reclaim sweep never retires below (runtime
    /// `types::Component::reclaim_min_instances`). Default 0 lets an idle pool
    /// empty out; capped at `poolSize` upstream and only meaningful alongside
    /// `reclaimWindowSeconds`.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub reclaim_min_instances: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_resources: Option<LocalResources>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn default_pool_size() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalResources {
    /// Desktop extensions — NOT in Control's CRD (its structural schema
    /// prunes them on `kubectl apply`); candidates for upstreaming into
    /// runtime-operator (docs/CONTROL-PATTERNS.md #2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_mb: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_limit: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<ConfigLayer>,
    /// Control: flat interface config map (distinct from `environment`).
    /// Preserved for round-trip portability.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub volume_mounts: Vec<VolumeMount>,
    /// Egress allowlist — fail-closed: empty/absent denies all outbound HTTP.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_hosts: Vec<String>,
    /// Ports on the machine's own loopback this component may reach through
    /// the `host.wasmcloud.internal` sentinel name (wash-runtime v2.7.0).
    /// Entries are `"PORT"` (TCP) or `"PORT/tcp"` / `"PORT/udp"` — no ranges,
    /// no wildcards, by upstream design. Fail-closed: empty/absent denies all,
    /// AND the grant is inert unless the host-level door is also open
    /// (`PUT /v1/egress` `allow_host_loopback`, default off) — neither the
    /// workload author nor the operator can open it alone. Same wire field as
    /// Control's CRD.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_host_loopback_ports: Vec<String>,
}

/// Layered config: literals + named config/secret sources, merged last-wins.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigLayer {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_from: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_from: Vec<SourceRef>,
}

/// By-name reference to a named config (`configFrom`) or registered secret ref
/// (`secretFrom`); resolved by the daemon at workload start.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Volume {
    pub name: String,
    /// Opaque CRD `ephemeral` volume-source payload, preserved verbatim for
    /// Control round-trip portability. The daemon never inspects its contents:
    /// any volume without `hostPath` is started as an ephemeral EmptyDir volume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_path: Option<HostPathVolume>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPathVolume {
    pub path: String,
}

/// Mounts a `spec.volumes` entry into the component's WASI filesystem at
/// `mount_path`; `read_only` defaults to false.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeMount {
    pub name: String,
    pub mount_path: String,
    #[serde(default)]
    pub read_only: bool,
}

impl Workload {
    pub fn new(name: &str, namespace: &str, spec: WorkloadSpec) -> Self {
        Self {
            api_version: API_VERSION.to_string(),
            kind: KIND_WORKLOAD.to_string(),
            metadata: Metadata {
                name: name.to_string(),
                namespace: namespace.to_string(),
                labels: BTreeMap::new(),
                annotations: BTreeMap::new(),
            },
            spec,
        }
    }

    /// The draft/dev single-component Workload shape shared by synth
    /// (paste-to-run drafts) and project mode (dev + publish): one component
    /// named after the workload, `pool_size` 1, no invocation cap, no service
    /// sidecar or volumes. The caller owns the egress decision via
    /// `local_resources` (dev allow-all vs publish deny-all).
    #[must_use]
    pub fn single_component(
        name: &str,
        namespace: &str,
        image: String,
        host_interfaces: Vec<HostInterface>,
        local_resources: Option<LocalResources>,
    ) -> Self {
        Self::new(
            name,
            namespace,
            WorkloadSpec {
                host_interfaces,
                components: vec![Component {
                    name: name.to_string(),
                    image,
                    image_pull_secret: None,
                    image_pull_policy: None,
                    pool_size: 1,
                    max_concurrency: 0,
                    reclaim_window_seconds: 0,
                    reclaim_min_instances: 0,
                    max_invocations: 0,
                    local_resources,
                }],
                ..Default::default()
            },
        )
    }

    pub fn enabled(&self) -> bool {
        self.metadata
            .annotations
            .get(ANNOTATION_ENABLED)
            .map(|v| v != "false")
            .unwrap_or(true)
    }

    /// Whether the explicit per-workload unsigned-image override is set
    /// (exact string `"true"` only — fail closed on anything else).
    pub fn unsafe_allow_unsigned(&self) -> bool {
        self.metadata
            .annotations
            .get(ANNOTATION_UNSAFE_ALLOW_UNSIGNED)
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.metadata
            .annotations
            .insert(ANNOTATION_ENABLED.to_string(), enabled.to_string());
    }

    /// Basic structural validation; returns human-actionable messages.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.api_version != API_VERSION {
            errs.push(format!(
                "unsupported apiVersion {:?} (expected {API_VERSION})",
                self.api_version
            ));
        }
        if self.kind != KIND_WORKLOAD {
            errs.push(format!(
                "unsupported kind {:?} (expected {KIND_WORKLOAD})",
                self.kind
            ));
        }
        if !is_dns_label(&self.metadata.name) {
            errs.push(format!(
                "metadata.name {:?} must be a DNS label",
                self.metadata.name
            ));
        }
        if !is_dns_label(&self.metadata.namespace) {
            errs.push(format!(
                "metadata.namespace {:?} must be a DNS label",
                self.metadata.namespace
            ));
        }
        if self.spec.components.is_empty() {
            errs.push("spec.components must not be empty".to_string());
        }
        for c in &self.spec.components {
            if c.name.is_empty() {
                errs.push("component name must not be empty".to_string());
            }
            if c.image.is_empty() {
                errs.push(format!("component {:?} has no image", c.name));
            }
        }
        if let Some(service) = &self.spec.service {
            if service.image.is_empty() {
                errs.push("spec.service has no image".to_string());
            }
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    /// Control-only fields present in this spec that Desktop preserves but
    /// does not act on (single host: no selector/environment scheduling; pull
    /// credentials come from the registry store, not k8s Secrets). The
    /// reconciler surfaces these as one Info event at apply so "it deployed
    /// but ignored my pin" is never silent.
    pub fn ignored_portability_fields(&self) -> Vec<&'static str> {
        let mut ignored = Vec::new();
        if !self.spec.host_selector.is_empty() {
            ignored.push("spec.hostSelector");
        }
        if self.spec.host_id.is_some() {
            ignored.push("spec.hostId");
        }
        if self.spec.environment.is_some() {
            ignored.push("spec.environment");
        }
        if self.spec.kubernetes.is_some() {
            ignored.push("spec.kubernetes");
        }
        if self
            .spec
            .components
            .iter()
            .any(|c| c.image_pull_secret.is_some())
            || self
                .spec
                .service
                .as_ref()
                .is_some_and(|s| s.image_pull_secret.is_some())
        {
            ignored.push("imagePullSecret");
        }
        ignored
    }
}

/// True if `s` is a valid DNS label: non-empty, ≤63 chars, lowercase alnum +
/// '-', no leading/trailing '-'. [`sanitize_dns_label`] coerces arbitrary
/// input into a string that satisfies this.
pub fn is_dns_label(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 63
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
}

/// Coerce an arbitrary string into a DNS label (lowercase alnum + '-', ≤63,
/// no leading/trailing '-'); falls back to "component" if nothing survives.
/// The output always satisfies [`is_dns_label`].
pub fn sanitize_dns_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len().min(63));
    let mut last_dash = true; // suppress leading dashes
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
        if out.len() == 63 {
            break;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "component".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Golden cross-product round-trip (docs/CONTROL-PATTERNS.md #2): a
    /// Control-authored spec using every runtime-operator v1alpha1 field must
    /// survive deserialize→serialize byte-for-value — the store re-serializes
    /// through these structs, so any field missing from them is silently
    /// STRIPPED from the user's spec. If this test fails after a
    /// runtime-operator bump, the schema drifted again: add the new field as
    /// pass-through (and extend `ignored_portability_fields` if Desktop
    /// doesn't act on it).
    #[test]
    fn control_authored_spec_round_trips_losslessly() {
        let control_spec = serde_json::json!({
            "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
            "kind": "Workload",
            "metadata": {
                "name": "portable-app",
                "namespace": "default",
                "labels": { "app.kubernetes.io/name": "portable-app" },
                "annotations": { "gitops.example.com/source": "repo" }
            },
            "spec": {
                "hostSelector": { "region": "us-east", "gpu": "true" },
                "hostId": "host-abc123",
                "environment": "staging",
                "kubernetes": { "service": { "name": "portable-app-svc" } },
                "hostInterfaces": [{
                    "name": "cache",
                    "namespace": "wasi",
                    "package": "keyvalue",
                    "interfaces": ["store"],
                    "config": { "backend": "filesystem", "root": "cache" }
                }],
                "components": [{
                    "name": "api",
                    "image": "ghcr.io/example/api:1.0.0",
                    "imagePullSecret": { "name": "ghcr-creds" },
                    "imagePullPolicy": "IfNotPresent",
                    "poolSize": 2,
                    "localResources": {
                        "config": { "mode": "fast" },
                        "allowedHosts": ["api.example.com"],
                        "environment": { "config": { "LOG_LEVEL": "info" } }
                    }
                }],
                "service": {
                    "image": "ghcr.io/example/sidecar:1.0.0",
                    "imagePullSecret": { "name": "ghcr-creds" },
                    "maxRestarts": 3
                }
            }
        });
        let parsed: Workload =
            serde_json::from_value(control_spec.clone()).expect("Control spec must parse");
        let reserialized = serde_json::to_value(&parsed).expect("serialize");
        // Every field Control wrote must still be there (Desktop may ADD
        // defaults like maxInvocations — that's fine; losing fields is not).
        fn assert_subset(expected: &serde_json::Value, actual: &serde_json::Value, path: &str) {
            match expected {
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        let sub = actual.get(k).unwrap_or_else(|| {
                            panic!("field {path}.{k} was DROPPED on round-trip (schema drift)")
                        });
                        assert_subset(v, sub, &format!("{path}.{k}"));
                    }
                }
                serde_json::Value::Array(items) => {
                    let actual_items = actual
                        .as_array()
                        .unwrap_or_else(|| panic!("field {path} changed shape on round-trip"));
                    assert_eq!(
                        items.len(),
                        actual_items.len(),
                        "array {path} changed length"
                    );
                    for (i, (e, a)) in items.iter().zip(actual_items).enumerate() {
                        assert_subset(e, a, &format!("{path}[{i}]"));
                    }
                }
                other => assert_eq!(other, actual, "field {path} changed value on round-trip"),
            }
        }
        assert_subset(&control_spec, &reserialized, "$");

        // And the ignored-fields report names what Desktop won't act on.
        let ignored = parsed.ignored_portability_fields();
        for expected in [
            "spec.hostSelector",
            "spec.hostId",
            "spec.environment",
            "spec.kubernetes",
            "imagePullSecret",
        ] {
            assert!(
                ignored.contains(&expected),
                "{expected} missing from {ignored:?}"
            );
        }
    }

    #[test]
    fn sanitize_always_yields_valid_dns_label() {
        let cases = [
            "hello-world",
            "Api_Gateway.wasm",
            "--weird--",
            "!!!",
            "",
            "123start-with-digit",
            "-leading-hyphen",
            "trailing-hyphen-",
            "MiXeD CaSe With Spaces",
            "café-déjà-vü", // unicode → stripped
            "日本語",       // all non-ascii → fallback
            "a.b.c.d.e.f.g",
            &"x".repeat(100),                    // very long → truncated to 63
            &"-".repeat(80),                     // all hyphens → fallback
            &format!("{}-tail", "z".repeat(62)), // truncation must not leave trailing '-'
        ];
        for input in cases {
            let label = sanitize_dns_label(input);
            assert!(
                is_dns_label(&label),
                "sanitize_dns_label({input:?}) = {label:?} is not a valid DNS label"
            );
        }
        // Documented fallback for input with nothing label-worthy.
        assert_eq!(sanitize_dns_label(""), "component");
        assert_eq!(sanitize_dns_label("!!!"), "component");
        // Length cap.
        assert_eq!(sanitize_dns_label(&"x".repeat(100)).len(), 63);
    }

    #[test]
    fn service_sidecar_round_trips_and_validates() {
        // A Workload with a `service:` section (the service-tcp template shape).
        // The on-disk format is YAML, but this crate only depends on
        // serde_json; the camelCase serde derives are identical, and the store
        // round-trips the same types through YAML (store.rs tests). The JSON
        // here uses the same camelCase keys the YAML does.
        let json = r#"{
            "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
            "kind": "Workload",
            "metadata": { "name": "tcp-echo", "namespace": "default" },
            "spec": {
                "components": [
                    { "name": "handler", "image": "ghcr.io/acme/handler:1.0.0" }
                ],
                "service": {
                    "name": "echo",
                    "image": "ghcr.io/acme/service-tcp:1.0.0",
                    "maxRestarts": 3,
                    "localResources": { "environment": { "config": { "PORT": "8080" } } }
                }
            }
        }"#;
        let w: Workload = serde_json::from_str(json).unwrap();
        w.validate().unwrap();
        let svc = w.spec.service.as_ref().expect("service present");
        assert_eq!(svc.name.as_deref(), Some("echo"));
        assert_eq!(svc.image, "ghcr.io/acme/service-tcp:1.0.0");
        assert_eq!(svc.max_restarts, Some(3));
        assert_eq!(
            svc.local_resources
                .as_ref()
                .unwrap()
                .environment
                .as_ref()
                .unwrap()
                .config
                .get("PORT")
                .map(String::as_str),
            Some("8080")
        );

        // Re-serialize and parse again: the service survives a full round trip,
        // and the wire key is camelCase `maxRestarts` (portable to Control).
        let serialized = serde_json::to_string(&w).unwrap();
        assert!(serialized.contains("\"maxRestarts\":3"), "{serialized}");
        let again: Workload = serde_json::from_str(&serialized).unwrap();
        assert_eq!(again, w);

        // A workload without a service still serializes with no `service` key.
        let mut plain = w.clone();
        plain.spec.service = None;
        let plain_json = serde_json::to_string(&plain).unwrap();
        assert!(!plain_json.contains("service"), "{plain_json}");

        // A service with no image is rejected.
        let mut bad = w.clone();
        bad.spec.service.as_mut().unwrap().image = String::new();
        assert!(bad.validate().is_err());
    }
}
