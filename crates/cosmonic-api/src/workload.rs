//! `runtime.wasmcloud.dev/v1alpha1` Workload spec — the subset Cosmonic
//! Desktop schedules (ARCHITECTURE.md §4). Field names and semantics match the
//! wasmCloud runtime-operator CRD so specs are portable to/from a Cosmonic
//! Control cluster. Desktop-local state (e.g. enabled) lives in annotations,
//! never in new fields.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const API_VERSION: &str = "runtime.wasmcloud.dev/v1alpha1";
pub const KIND_WORKLOAD: &str = "Workload";
/// Cosmonic Control's deployment kind. Cosmonic Desktop runs the workload it
/// wraps rather than the deployment itself, so a manifest written for Control
/// applies here unchanged. See [`Workload::from_manifest`].
pub const KIND_WORKLOAD_DEPLOYMENT: &str = "WorkloadDeployment";

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
/// The kind a workload was read out of, when that was not `Workload`. Set by
/// [`Workload::from_deployment`]; read by [`Workload::ignored_portability_fields`]
/// and by anything reconstructing the manifest a user actually wrote.
pub const ANNOTATION_FROM_KIND: &str = "desktop.cosmonic.com/from-kind";
/// `spec.replicas` as the deployment asked for it. A single host runs one, so
/// this is a record rather than an instruction.
pub const ANNOTATION_REPLICAS: &str = "desktop.cosmonic.com/replicas";
/// Comma-separated cluster-only fields the envelope carried that this host
/// cannot act on, so the apply-notes event can name them.
pub const ANNOTATION_UNSUPPORTED_FIELDS: &str = "desktop.cosmonic.com/unsupported-fields";

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

/// A Cosmonic Control `WorkloadDeployment` document.
///
/// Typed rather than hand-walked for four reasons: a derived `Deserialize`
/// refuses a duplicate key where a `Value` walk takes the last silently; the
/// same code reads YAML and JSON, so synthesis needs no cross-format
/// conversion; serde's errors name the full path; and a field named here is a
/// field that cannot be dropped without someone noticing.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadDeploymentDoc {
    pub api_version: String,
    pub kind: String,
    /// Defaulted so a document with no `metadata` block reports one missing
    /// name rather than a missing name and an empty namespace.
    #[serde(default = "default_metadata")]
    pub metadata: Metadata,
    pub spec: WorkloadDeploymentSpec,
}

/// `WorkloadDeployment.spec`: the replica count, the workload template, and
/// the two cluster-only fields Desktop records but cannot act on.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadDeploymentSpec {
    /// Hosts a cluster would run this on. Named as a signed integer so a
    /// negative or fractional value is a parse error the author can read,
    /// rather than falling through an `as_u64` and being ignored.
    #[serde(default)]
    pub replicas: Option<i64>,
    pub template: WorkloadReplicaTemplate,
    /// A cluster's rollout strategy. Preserved for reporting only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deploy_policy: Option<serde_json::Value>,
    /// `Artifact` references a cluster resolves into component images. Desktop
    /// has no Artifact controller, so a spec that binds its image this way is
    /// reported rather than silently starting without one.
    #[serde(default)]
    pub artifacts: Vec<serde_json::Value>,
    /// The operator's EndpointSlice and Service DNS aliases. A manifest whose
    /// reachability story is Service DNS has none of it here, so this is
    /// reported rather than dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubernetes: Option<serde_json::Value>,
}

/// `WorkloadDeployment.spec.template`. Note the labels and annotations are
/// flat here, not under a nested `metadata` as k8s habit suggests.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadReplicaTemplate {
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub annotations: BTreeMap<String, String>,
    /// Not a field this template has. Captured only so a document written from
    /// Kubernetes `Deployment` habit, where labels live under
    /// `template.metadata`, is told they were ignored instead of losing them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub spec: WorkloadSpec,
}

fn default_metadata() -> Metadata {
    Metadata {
        namespace: default_namespace(),
        ..Metadata::default()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub host_interfaces: Vec<HostInterface>,
    /// May be omitted (or empty) when `service` is set. Always serialized, so
    /// a client reading the wire always finds the array.
    #[serde(default)]
    pub components: Vec<Component>,
    /// Optional long-running **service** (the wasmCloud v2 Workload's
    /// `service`): a component exporting `wasi:cli/run`, which the runtime
    /// drives for the life of the workload — a component under `components`
    /// is only ever invoked, so its `run` is never called. It may own
    /// localhost ports (e.g. the service-tcp template) or a plugin session
    /// (the Kafka pull-service starter). A workload may carry a service and
    /// no components at all. Drives the UI Services tab.
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

/// A workload's service — matches the runtime-operator CRD / `wash_runtime`
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

impl WorkloadService {
    /// The name the service is keyed by wherever it sits beside components —
    /// status `componentInterfaces` and `imageDigests`, revision artifacts,
    /// the inspect graph: its `name`, else `"service"`.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("service")
    }
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
    /// or volumes. The caller owns the egress decision via
    /// `local_resources` (dev allow-all vs publish deny-all). Callers use
    /// [`Workload::single_service`] only when the user chose the service role;
    /// a `wasi:cli/run` export alone is ambiguous.
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

    /// [`Workload::single_component`]'s counterpart for an artifact that runs
    /// as the workload's **service** (it exports `wasi:cli/run`): no
    /// components, one unnamed service with the runtime's restart default.
    /// The service is left unnamed on purpose — `name` is a Desktop display
    /// field Control's CRD does not carry, and a draft must stay portable.
    #[must_use]
    pub fn single_service(
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
                service: Some(WorkloadService {
                    name: None,
                    image,
                    image_pull_secret: None,
                    image_pull_policy: None,
                    max_restarts: None,
                    local_resources,
                }),
                ..Default::default()
            },
        )
    }

    /// Every runnable's image, components first then the service — the set
    /// a pull, a pin or an image list walks.
    pub fn images(&self) -> impl Iterator<Item = &str> {
        self.spec
            .components
            .iter()
            .map(|c| c.image.as_str())
            .chain(self.spec.service.iter().map(|s| s.image.as_str()))
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

    /// Build a workload from a Control deployment envelope.
    ///
    /// A `WorkloadDeployment` wraps the same spec a `Workload` carries, under
    /// `.spec.template.spec`, with `replicas` beside it. The two describe the
    /// same workload, so a manifest written for Cosmonic Control runs here and
    /// the envelope is unwrapped rather than refused.
    ///
    /// What the envelope carries and one host does not is recorded on the
    /// workload, not dropped: `replicas` and the kind it came from become
    /// annotations, so [`Workload::ignored_portability_fields`] reports them
    /// at apply through the same Info event that already covers `hostSelector`
    /// and friends, and a round trip back to a cluster can see what was lost.
    /// Annotations are outside the revision spec hash, so this adds no churn.
    ///
    /// `replicas: 0` scales a deployment to nothing on a cluster. The nearest
    /// honest thing here is a workload that exists and does not run, which is
    /// what `enabled: false` already means, so the workload is applied stopped
    /// rather than started against the author's intent.
    ///
    /// `HTTPTrigger` is deliberately not handled here. It is a translation
    /// rather than an envelope: an ingress binding has to be derived from
    /// `spec.ingress.host`, and inventing a hostInterface during an apply is a
    /// bigger surprise than dropping `replicas`. That belongs in synthesis,
    /// where the user reviews the derived spec before it is applied.
    pub fn from_deployment(doc: WorkloadDeploymentDoc) -> (Self, Vec<String>) {
        let mut notes = Vec::new();
        let WorkloadDeploymentDoc {
            api_version,
            kind,
            mut metadata,
            spec,
        } = doc;

        // The template's labels and annotations describe the workloads the
        // deployment creates; the deployment's describe the deployment. The
        // more specific one wins where both name the same key.
        metadata.labels.extend(spec.template.labels);
        metadata.annotations.extend(spec.template.annotations);

        let mut workload = Self {
            api_version,
            kind: KIND_WORKLOAD.to_string(),
            metadata,
            spec: spec.template.spec,
        };
        workload
            .metadata
            .annotations
            .insert(ANNOTATION_FROM_KIND.to_string(), kind);

        // Two cluster-only fields with no local equivalent. `artifacts` is the
        // costly one: a spec that binds its image through an `Artifact` has no
        // image here, and "component has no image" names nothing a reader can
        // act on without this.
        let mut unsupported = Vec::new();
        if !spec.artifacts.is_empty() {
            unsupported.push("spec.artifacts");
        }
        if spec.deploy_policy.is_some() {
            unsupported.push("spec.deployPolicy");
        }
        if spec.kubernetes.is_some() {
            unsupported.push("spec.kubernetes");
        }
        if !spec.artifacts.is_empty() {
            notes.push(format!(
                "spec.artifacts names {} Artifact reference(s), which a cluster resolves into \
                 component images. This host has no Artifact controller: set `image` on each \
                 component instead.",
                spec.artifacts.len()
            ));
        }
        if spec.template.metadata.is_some() {
            notes.push(
                "spec.template.metadata is not a field here. A WorkloadDeployment carries its \
                 labels and annotations directly on spec.template, so anything under \
                 spec.template.metadata was ignored."
                    .to_string(),
            );
        }
        if spec.kubernetes.is_some() {
            notes.push(
                "spec.kubernetes is the operator's EndpointSlice and Service DNS. A single host \
                 has neither: reach this workload through its wasi:http host config instead."
                    .to_string(),
            );
        }
        if spec.deploy_policy.is_some() {
            notes.push(
                "spec.deployPolicy is a cluster's rollout strategy and does not apply to a \
                 single host."
                    .to_string(),
            );
        }

        if !unsupported.is_empty() {
            workload.metadata.annotations.insert(
                ANNOTATION_UNSUPPORTED_FIELDS.to_string(),
                unsupported.join(","),
            );
        }

        if let Some(replicas) = spec.replicas {
            workload
                .metadata
                .annotations
                .insert(ANNOTATION_REPLICAS.to_string(), replicas.to_string());
            if replicas == 0 {
                workload.set_enabled(false);
                notes.push(
                    "replicas: 0 means a cluster runs none of these, so this is applied \
                     stopped. Start it when you want it running."
                        .to_string(),
                );
            } else if replicas != 1 {
                notes.push(format!(
                    "replicas: {replicas} asks a cluster for {replicas} hosts. This host is \
                     one, so one instance runs. Use poolSize for local concurrency."
                ));
            }
        }

        (workload, notes)
    }

    /// Read either kind this host can run from a JSON manifest body.
    ///
    /// A `Workload` is the flat shape the daemon stores; a
    /// `WorkloadDeployment` is [`Workload::from_deployment`]. Anything else,
    /// including a missing or unknown kind, is read as a flat `Workload` so
    /// that [`Workload::validate`] reports the kind itself rather than serde
    /// reporting a field the author never meant to write.
    ///
    /// Reads from the raw body, not a pre-parsed `serde_json::Value`, so that
    /// a duplicate key is still refused. `serde_json::Value` merges duplicates
    /// last-wins before any derived `Deserialize` sees them, and on an
    /// endpoint whose namespace decides an authorization guard, the parse that
    /// decides which `metadata` wins should be the strict one.
    pub fn from_manifest_slice(body: &[u8]) -> Result<(Self, Vec<String>), Vec<String>> {
        #[derive(Deserialize)]
        struct KindOnly {
            #[serde(default)]
            kind: String,
        }
        let peek: KindOnly = serde_json::from_slice(body)
            .map_err(|e| vec![format!("body is not a JSON manifest: {e}")])?;
        if peek.kind == KIND_WORKLOAD_DEPLOYMENT {
            return serde_json::from_slice::<WorkloadDeploymentDoc>(body)
                .map(Self::from_deployment)
                .map_err(|e| vec![format!("invalid {KIND_WORKLOAD_DEPLOYMENT} manifest: {e}")]);
        }
        serde_json::from_slice::<Self>(body)
            .map(|w| (w, Vec::new()))
            .map_err(|e| vec![format!("invalid {KIND_WORKLOAD} manifest: {e}")])
    }

    /// [`Workload::from_manifest_slice`] for a value already in hand. Callers
    /// holding raw bytes should prefer the slice form, which is stricter.
    pub fn from_manifest(value: serde_json::Value) -> Result<(Self, Vec<String>), Vec<String>> {
        let body = serde_json::to_vec(&value)
            .map_err(|e| vec![format!("manifest is not serializable: {e}")])?;
        Self::from_manifest_slice(&body)
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
        // A service-only workload is valid (the runtime drives the service
        // and invokes no component); a workload with neither runs nothing.
        if self.spec.components.is_empty() && self.spec.service.is_none() {
            errs.push("spec must declare at least one component or a service".to_string());
        }
        for c in &self.spec.components {
            if c.name.is_empty() {
                errs.push("component name must not be empty".to_string());
            }
            if c.image.is_empty() {
                errs.push(format!("component {:?} has no image", c.name));
            }
        }
        let names = self.spec.components.iter().map(|c| c.name.as_str());
        if let Some(name) = duplicate_component_name(names) {
            errs.push(duplicate_component_message(name));
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
        // Recorded by `from_deployment` rather than present in the spec: a
        // replica count a single host cannot honour. `1` is what this host
        // does, so it is not worth an event.
        if self
            .metadata
            .annotations
            .get(ANNOTATION_REPLICAS)
            .is_some_and(|r| r != "1")
        {
            ignored.push("spec.replicas");
        }
        // Whatever else the deployment envelope carried. Static strings, so
        // this matches on the two the envelope can name rather than echoing
        // the annotation back.
        if let Some(fields) = self.metadata.annotations.get(ANNOTATION_UNSUPPORTED_FIELDS) {
            for field in fields.split(',') {
                match field.trim() {
                    "spec.artifacts" => ignored.push("spec.artifacts"),
                    "spec.deployPolicy" => ignored.push("spec.deployPolicy"),
                    "spec.kubernetes" => ignored.push("spec.kubernetes"),
                    _ => {}
                }
            }
        }
        ignored
    }
}

/// The first component name that appears more than once, if any.
///
/// A workload's grants name its components (`components: [<name>]` on a
/// session, chat or inference binding), so two components of one name would
/// both match: a second `{name: "agent", image: <anything>}` would be handed
/// the agent's conversation store and inference. Every path that accepts a
/// workload refuses one (apply, a washlet push, and the start-time mapping
/// for a spec stored before this check).
pub fn duplicate_component_name<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = std::collections::HashSet::new();
    names.into_iter().find(|n| !seen.insert(*n))
}

/// The refusal for [`duplicate_component_name`].
pub fn duplicate_component_message(name: &str) -> String {
    format!(
        "component name {name:?} is used more than once; each component of a workload needs its \
         own name, since grants name components"
    )
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

    #[test]
    fn a_service_only_workload_is_valid_and_an_empty_one_is_not() {
        // The Kafka pull-service shape: a `wasi:cli/run` component and nothing
        // invoked alongside it. `components` may be omitted entirely.
        let json = r#"{
            "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
            "kind": "Workload",
            "metadata": { "name": "pipeline", "namespace": "default" },
            "spec": {
                "service": { "image": "ghcr.io/acme/pull-service:1.0.0", "maxRestarts": 100 }
            }
        }"#;
        let w: Workload = serde_json::from_str(json).unwrap();
        assert!(w.spec.components.is_empty());
        w.validate().unwrap();

        let built = Workload::single_service(
            "pipeline",
            "default",
            "ghcr.io/acme/pull-service:1.0.0".into(),
            Vec::new(),
            None,
        );
        built.validate().unwrap();
        let svc = built.spec.service.as_ref().unwrap();
        assert_eq!(svc.name, None, "a draft's service stays unnamed (portable)");
        assert_eq!(
            built.images().collect::<Vec<_>>(),
            ["ghcr.io/acme/pull-service:1.0.0"]
        );

        let mut empty = w;
        empty.spec.service = None;
        let errs = empty.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("at least one component or a service")),
            "{errs:?}"
        );
    }

    /// Two components of one name would both match a grant naming it, so a
    /// workload carrying them is refused.
    #[test]
    fn a_workload_with_two_components_of_one_name_is_refused() {
        let json = r#"{
            "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
            "kind": "Workload",
            "metadata": { "name": "agent-session-x", "namespace": "default" },
            "spec": {
                "components": [
                    { "name": "agent", "image": "ghcr.io/cosmonic/agents/goose:1.0.0" },
                    { "name": "tools", "image": "ghcr.io/cosmonic/agents/tools:1.0.0" },
                    { "name": "agent", "image": "ghcr.io/evil/impostor:1.0.0" }
                ]
            }
        }"#;
        let w: Workload = serde_json::from_str(json).unwrap();
        let errs = w.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("\"agent\" is used more than once")), "{errs:?}");
        let mut unique = w;
        unique.spec.components.pop();
        unique.validate().unwrap();
        assert_eq!(duplicate_component_name(["a", "b", "a"]), Some("a"));
        assert_eq!(duplicate_component_name(["a", "b"]), None);
    }

    /// The shape every Control manifest, kubectl example and template's
    /// Kubernetes file has. Refusing it was the most common way an apply
    /// failed, so these pin the acceptance rather than the refusal.
    mod from_manifest {
        use super::super::*;
        use serde_json::json;

        fn deployment(spec_extra: serde_json::Value) -> serde_json::Value {
            let mut spec = json!({
                "replicas": 1,
                "template": {
                    "spec": {
                        "components": [{ "name": "app", "image": "ghcr.io/x/app:1" }]
                    }
                }
            });
            if let (Some(a), Some(b)) = (spec.as_object_mut(), spec_extra.as_object()) {
                for (k, v) in b {
                    a.insert(k.clone(), v.clone());
                }
            }
            json!({
                "apiVersion": API_VERSION,
                "kind": KIND_WORKLOAD_DEPLOYMENT,
                "metadata": { "name": "app", "namespace": "default" },
                "spec": spec,
            })
        }

        #[test]
        fn unwraps_a_deployment_into_a_runnable_workload() {
            let (w, notes) = Workload::from_manifest(deployment(json!({}))).unwrap();
            assert_eq!(w.kind, KIND_WORKLOAD);
            assert_eq!(w.metadata.name, "app");
            assert_eq!(w.spec.components[0].image, "ghcr.io/x/app:1");
            w.validate().expect("the unwrapped workload is applyable");
            // The conversion is recorded on the workload, not announced as a
            // warning: a warning on the happy path teaches people to skip
            // warnings, and a replicas:1 Control manifest is the happy path.
            assert_eq!(
                w.metadata
                    .annotations
                    .get(ANNOTATION_FROM_KIND)
                    .map(String::as_str),
                Some(KIND_WORKLOAD_DEPLOYMENT)
            );
            assert!(notes.is_empty(), "{notes:?}");
        }

        #[test]
        fn a_flat_workload_is_unchanged_and_gains_no_notes() {
            let flat = json!({
                "apiVersion": API_VERSION,
                "kind": KIND_WORKLOAD,
                "metadata": { "name": "app", "namespace": "default" },
                "spec": { "components": [{ "name": "app", "image": "ghcr.io/x/app:1" }] },
            });
            let (w, notes) = Workload::from_manifest(flat).unwrap();
            assert_eq!(w.kind, KIND_WORKLOAD);
            assert!(notes.is_empty(), "{notes:?}");
        }

        #[test]
        fn replicas_beyond_one_are_recorded_and_reported_at_apply() {
            let (w, notes) = Workload::from_manifest(deployment(json!({ "replicas": 3 }))).unwrap();
            assert_eq!(
                w.metadata
                    .annotations
                    .get(ANNOTATION_REPLICAS)
                    .map(String::as_str),
                Some("3")
            );
            // The channel apply actually reads. Notes alone never reach it.
            assert!(
                w.ignored_portability_fields().contains(&"spec.replicas"),
                "{:?}",
                w.ignored_portability_fields()
            );
            assert!(
                notes.iter().any(|n| n.contains("one instance runs")),
                "{notes:?}"
            );
        }

        #[test]
        fn one_replica_is_what_this_host_does_and_earns_no_event() {
            let (w, notes) = Workload::from_manifest(deployment(json!({ "replicas": 1 }))).unwrap();
            assert!(!w.ignored_portability_fields().contains(&"spec.replicas"));
            assert!(notes.is_empty(), "{notes:?}");
            assert!(w.enabled());
        }

        #[test]
        fn zero_replicas_applies_the_workload_stopped() {
            let (w, notes) = Workload::from_manifest(deployment(json!({ "replicas": 0 }))).unwrap();
            assert!(
                !w.enabled(),
                "replicas: 0 asks a cluster to run none of these"
            );
            assert!(notes.iter().any(|n| n.contains("applied")), "{notes:?}");
        }

        /// `as_u64` silently ignored every one of these, which are exactly the
        /// inputs odd enough to deserve an answer.
        #[test]
        fn a_replica_count_that_is_not_a_whole_number_is_an_error() {
            for bad in [json!("0"), json!(2.5), json!(-1)] {
                let doc = deployment(json!({ "replicas": bad.clone() }));
                let r = Workload::from_manifest(doc);
                if bad == json!(-1) {
                    // Parses as an integer; caught as a count, not a type.
                    let (_, notes) = r.expect("-1 parses");
                    assert!(
                        notes.iter().any(|n| n.contains("one instance runs")),
                        "{notes:?}"
                    );
                } else {
                    assert!(r.is_err(), "replicas: {bad} should not be silently ignored");
                }
            }
        }

        /// A `Value` walk takes the last duplicate silently. On an endpoint
        /// guarded by namespace, that is a parser differential.
        /// `serde_json::Value` merges duplicates last-wins before any derived
        /// `Deserialize` runs, so reading from a Value would silently pick a
        /// namespace the author did not write. The slice form is the strict
        /// one, and it is what the HTTP handlers use.
        #[test]
        fn a_duplicate_key_is_refused_rather_than_last_wins() {
            let raw = r#"{"apiVersion":"runtime.wasmcloud.dev/v1alpha1","kind":"WorkloadDeployment",
                "metadata":{"name":"app","namespace":"sys-platform"},
                "metadata":{"name":"app","namespace":"default"},
                "spec":{"template":{"spec":{"components":[{"name":"a","image":"x:1"}]}}}}"#;
            // What a Value-based read would have decided: the last one.
            let lenient: serde_json::Value = serde_json::from_str(raw).expect("json parses");
            assert_eq!(lenient["metadata"]["namespace"], "default");

            // What the handlers actually do.
            let errs = Workload::from_manifest_slice(raw.as_bytes()).unwrap_err();
            assert!(errs[0].contains("duplicate"), "{errs:?}");
        }

        #[test]
        fn a_flat_workload_with_a_duplicate_key_is_refused_too() {
            let raw = r#"{"apiVersion":"runtime.wasmcloud.dev/v1alpha1","kind":"Workload",
                "metadata":{"name":"a","namespace":"sys-platform"},
                "metadata":{"name":"a","namespace":"default"},
                "spec":{"components":[{"name":"a","image":"x:1"}]}}"#;
            let errs = Workload::from_manifest_slice(raw.as_bytes()).unwrap_err();
            assert!(errs[0].contains("duplicate"), "{errs:?}");
        }

        /// The envelope must not launder an apiVersion past the check that
        /// exists to catch it.
        #[test]
        fn a_deployment_without_an_apiversion_is_still_judged() {
            let doc = json!({
                "kind": KIND_WORKLOAD_DEPLOYMENT,
                "metadata": { "name": "app", "namespace": "default" },
                "spec": { "template": { "spec": {
                    "components": [{ "name": "app", "image": "x:1" }] } } },
            });
            assert!(
                Workload::from_manifest(doc).is_err(),
                "a missing apiVersion must not be defaulted into validity"
            );
        }

        #[test]
        fn the_k8s_habit_location_for_labels_is_reported() {
            let mut doc = deployment(json!({}));
            doc["spec"]["template"]["metadata"] = json!({ "labels": { "a": "b" } });
            let (w, notes) = Workload::from_manifest(doc).unwrap();
            assert!(w.metadata.labels.is_empty());
            assert!(
                notes.iter().any(|n| n.contains("spec.template.metadata")),
                "{notes:?}"
            );
        }

        #[test]
        fn the_operators_service_dns_is_reported() {
            let doc = deployment(json!({ "kubernetes": { "service": { "name": "x" } } }));
            let (w, notes) = Workload::from_manifest(doc).unwrap();
            assert!(w.ignored_portability_fields().contains(&"spec.kubernetes"));
            assert!(notes.iter().any(|n| n.contains("Service DNS")), "{notes:?}");
        }

        #[test]
        fn the_cluster_only_fields_are_reported_not_dropped() {
            let doc = deployment(json!({
                "artifacts": [{ "name": "a" }],
                "deployPolicy": { "strategy": "RollingUpdate" }
            }));
            let (w, notes) = Workload::from_manifest(doc).unwrap();
            let ignored = w.ignored_portability_fields();
            assert!(ignored.contains(&"spec.artifacts"), "{ignored:?}");
            assert!(ignored.contains(&"spec.deployPolicy"), "{ignored:?}");
            assert!(notes.iter().any(|n| n.contains("Artifact")), "{notes:?}");
        }

        #[test]
        fn the_kind_it_came_from_is_kept_for_the_round_trip() {
            let (w, _) = Workload::from_manifest(deployment(json!({}))).unwrap();
            assert_eq!(
                w.metadata
                    .annotations
                    .get(ANNOTATION_FROM_KIND)
                    .map(String::as_str),
                Some(KIND_WORKLOAD_DEPLOYMENT)
            );
        }

        /// A non-string label used to vanish, letting the deployment's value
        /// win where the doc promises the template's does.
        #[test]
        fn a_non_string_label_is_an_error_not_a_silent_fallback() {
            let mut doc = deployment(json!({}));
            doc["spec"]["template"]["labels"] = json!({ "tier": 5 });
            assert!(Workload::from_manifest(doc).is_err());
        }

        #[test]
        fn a_deployment_without_the_nesting_says_which_level_is_missing() {
            let doc = json!({
                "apiVersion": API_VERSION,
                "kind": KIND_WORKLOAD_DEPLOYMENT,
                "metadata": { "name": "app" },
                "spec": { "replicas": 1 },
            });
            let errs = Workload::from_manifest(doc).unwrap_err();
            // serde names the missing field and its path, which is better
            // than the three hand-written sentences this used to carry.
            assert!(errs[0].contains("missing field `template`"), "{errs:?}");
        }

        #[test]
        fn an_unknown_kind_still_reports_the_kind_rather_than_serde() {
            let doc = json!({
                "apiVersion": API_VERSION,
                "kind": "HTTPTrigger",
                "metadata": { "name": "app", "namespace": "default" },
                "spec": { "components": [{ "name": "app", "image": "ghcr.io/x/app:1" }] },
            });
            let (w, _) = Workload::from_manifest(doc).unwrap();
            let errs = w.validate().unwrap_err();
            assert!(
                errs.iter().any(|e| e.contains("unsupported kind")),
                "{errs:?}"
            );
        }

        /// Rust eats the newline and the next line's indentation after a `\`,
        /// so these read as one sentence. The sibling guard in api.rs covers
        /// the same hazard for validate's warnings.
        #[test]
        fn the_messages_are_single_spaced() {
            let doc = json!({
                "apiVersion": API_VERSION,
                "kind": KIND_WORKLOAD_DEPLOYMENT,
                "metadata": { "name": "app" },
                "spec": { "template": {} },
            });
            let errs = Workload::from_manifest(doc).unwrap_err();
            assert!(!errs[0].contains("  "), "{:?}", errs[0]);
        }
    }
}
