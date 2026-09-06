//! MCP resource payloads. Dynamic resources (host, workloads, templates,
//! catalog) are fetched from the daemon at read time in `mod.rs`; the Workload
//! **schema** below is static grounding so the agent emits valid specs.

use serde_json::{json, Value};

/// `cosmonic://schema/workload` — the spec shape + a worked example. This is the
/// single highest-leverage resource: it lets the model author a correct
/// `cosmonic_workload_apply` argument without guessing field names. Mirrors
/// `cosmonic-api::workload` (camelCase wire form).
pub const WORKLOAD_SCHEMA_DOC: &str = r#"# Cosmonic Workload spec (runtime.wasmcloud.dev/v1alpha1)

The unit of deployment. JSON (camelCase) accepted by `cosmonic_workload_apply`
and `POST /v1/workloads`. The daemon pins `image` to a digest at apply and
cosign-verifies it per policy on every start.

## Fields
- `apiVersion`: always `"runtime.wasmcloud.dev/v1alpha1"`
- `kind`: always `"Workload"`
- `metadata.name`: DNS label (lowercase alnum + `-`, <=63). Required.
- `metadata.namespace`: DNS label, default `"default"`.
- `metadata.annotations`: map. `desktop.cosmonic.com/enabled: "true"` keeps it
  running (apply enables by default).
- `spec.components[]`: at least one.
  - `name`: DNS label. `image`: OCI ref (e.g. `ghcr.io/acme/api:0.1.0`).
  - `poolSize`: warm instances parked between calls (default 1; 0 = a fresh
    instance per call). `maxInvocations`: calls one warm instance serves before
    retiring, 0 = unlimited. `maxConcurrency`: calls one warm instance serves
    at the same time (default 0 = one at a time; raise only for guests that
    yield rather than block). **Caveat:** on the HTTP ingress these only take
    effect for P3 components (exporting `wasi:http/handler@0.3.0`, bound as
    `interfaces: ["handler"]`): that is what the `rust-http`, `rust-mcp` and
    `go-http` templates produce. A P2 `incoming-handler` component (`ts-http`,
    most upstream images) accepts `poolSize` and ignores it. Do not tell the
    user you tuned throughput by setting `poolSize` on a P2 component; you did
    not. `cosmonic_image_inspect` shows which kind an image is.
  - `localResources.environment.config`: literal env vars (map).
  - `localResources.environment.secretFrom[]`: `{ name }` of a registered secret
    ref (never inline secret values; register with `cosmonic_secret_set`).
  - `localResources.allowedHosts[]`: outbound allow-list. **Empty = deny-all
    (fail-closed).** You MUST list every host the component dials out to.
  - `localResources.allowedHostLoopbackPorts[]`: ports on the machine's own
    loopback the component may reach via the `host.wasmcloud.internal`
    hostname (a local database, Ollama). Entries `"5432"` (TCP) or
    `"514/udp"`, with no ranges/wildcards. Empty = deny-all, AND inert unless the
    host operator also enabled loopback grants (Settings → Security); do not
    promise it works without that.
- `spec.hostInterfaces[]`: capabilities the workload uses/exports. For an HTTP
  API, expose the handler so the ingress routes to it:
  `{ namespace: "wasi", package: "http", interfaces: ["handler"],
     config: { host: "<name>.localhost" } }` for a P3 component (`rust-http`,
  `rust-mcp`, `go-http`): that is both what routes it and what puts it on the
  warm-instance pool. Use `interfaces: ["incoming-handler"]` for a P2 one
  (`ts-http`, most upstream images). Always
  run `cosmonic_image_inspect` on the image to see its real imports/exports rather
  than guessing which of the two it is.
  For a component importing `wasmcloud:secrets` (store/reveal), declare
  `{ namespace: "wasmcloud", package: "secrets", interfaces: ["store", "reveal"],
     secretFrom: [{ name: "<ref>" }] }`: the daemon resolves the refs at
  start and the component receives opaque secret handles it `reveal`s on
  use; the values never enter its environment. Prefer this over
  `localResources.environment.secretFrom` when the component supports it.

## Worked example: an HTTP API on the local ingress (127.0.0.1:8200)
```json
{
  "apiVersion": "runtime.wasmcloud.dev/v1alpha1",
  "kind": "Workload",
  "metadata": { "name": "time-api", "namespace": "default" },
  "spec": {
    "hostInterfaces": [
      { "namespace": "wasi", "package": "http", "interfaces": ["incoming-handler"],
        "config": { "host": "time-api.localhost" } }
    ],
    "components": [
      { "name": "api", "image": "ghcr.io/acme/time-api:0.1.0", "poolSize": 1,
        "localResources": {
          "environment": { "config": { "LOG_LEVEL": "info" } },
          "allowedHosts": []
        } }
    ]
  }
}
```
Reach it at `http://time-api.localhost:8200/` (routed by the Host header), or
`curl -H 'Host: time-api.localhost' http://127.0.0.1:8200/`.
"#;

/// MIME type for a `cosmonic://` resource.
///
/// `resources/list` has to report the same type `resources/read` returns, or a
/// client that keys off it re-parses the body. Everything dynamic is the
/// daemon's JSON; the schema doc is Markdown.
pub fn mime_for(uri: &str) -> &'static str {
    match uri {
        "cosmonic://schema/workload" => "text/markdown",
        _ => "application/json",
    }
}

/// Compose `cosmonic://capabilities` — what THIS host can actually run.
///
/// The highest-value grounding the server can offer, because it is the part no
/// external documentation can get right: the wash-runtime this build embeds,
/// which HTTP worlds it serves and the exact `hostInterfaces` binding each one
/// needs, whether the loopback door is open, and what the signature policy will
/// do on start. Models reliably guess the p2/p3 distinction wrong and produce a
/// workload that starts and never serves; this states it, per host, as fact.
///
/// Every input is optional. A daemon that cannot answer one of these — an older
/// build without the route — omits that section rather than failing the read:
/// partial grounding is worth more than none, and a missing section is visible.
pub fn capabilities(
    host: Option<&Value>,
    policy: Option<&Value>,
    egress: Option<&Value>,
    nats: Option<&Value>,
) -> Value {
    let mut out = serde_json::Map::new();

    if let Some(host) = host {
        let addr = host
            .get("httpAddr")
            .or_else(|| host.get("http_addr"))
            .and_then(|a| a.as_str());
        out.insert(
            "host".into(),
            json!({
                "daemonVersion": host.get("version"),
                "runtimeVersion": host.get("runtimeVersion"),
                "os": host.get("os"),
                "devBuild": host.get("devBuild"),
                "ingressBaseUrl": addr.map(|a| format!("http://{a}")),
                "messaging": host.get("messaging"),
            }),
        );
    }

    // Static for this build, and the single most useful thing here.
    out.insert("http".into(), json!({
        "routing": "The ingress routes by HTTP Host header, taken from a wasi:http hostInterface's `config.host`. Reach a workload with `curl -H 'Host: <name>.localhost' <ingressBaseUrl>/`.",
        "worlds": [
            {
                "world": "wasi:http/handler@0.3.0",
                "generation": "p3",
                "hostInterface": { "namespace": "wasi", "package": "http", "interfaces": ["handler"] },
                "warmPool": true,
                "producedBy": ["rust-http", "rust-mcp", "go-http"],
                "note": "poolSize, maxConcurrency and maxInvocations take effect for this world."
            },
            {
                "world": "wasi:http/incoming-handler@0.2",
                "generation": "p2",
                "hostInterface": { "namespace": "wasi", "package": "http", "interfaces": ["incoming-handler"] },
                "warmPool": false,
                "producedBy": ["ts-http", "most upstream images"],
                "note": "poolSize and maxConcurrency are ACCEPTED AND IGNORED for this world. Do not report that throughput was tuned by setting them here."
            }
        ],
        "howToTell": "Run `cosmonic_image_inspect` on the image and read its exports. Binding the wrong one produces a workload that starts and never serves — the most common failure on this host.",
    }));

    let loopback = egress
        .and_then(|e| {
            e.pointer("/active/allow_host_loopback")
                .or_else(|| e.pointer("/settings/allow_host_loopback"))
        })
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    out.insert("egress".into(), json!({
        "default": "deny-all",
        "note": "A component reaches only the hosts in its `spec.components[].localResources.allowedHosts`. An empty list denies everything, so a missing host surfaces as a connection failure rather than a permission error.",
        "hostLoopback": {
            "enabled": loopback,
            "note": if loopback {
                "`allowedHostLoopbackPorts` entries are honoured; a component reaches the machine's own loopback via the `host.wasmcloud.internal` hostname."
            } else {
                "The loopback door is CLOSED on this host, so `allowedHostLoopbackPorts` is inert. Do not promise a component can reach a local database or Ollama until the operator enables it in Settings → Security."
            },
        },
        "trustRoots": egress.and_then(|e| e.pointer("/active/trust_roots").cloned()),
    }));

    if let Some(nats) = nats {
        let active = nats.get("active").unwrap_or(nats);
        out.insert("nats".into(), json!({
            "servers": active.get("servers"),
            "workloadConfig": active.get("workload_config"),
            "hostInterface": { "namespace": "wasmcloud", "package": "nats" },
            "note": "The host owns the connection: a workload manifest naming its own NATS address is refused. Subjects, streams and buckets are granted per workload and deny-by-default.",
            "skill": "skill://cosmonic-nats/SKILL.md",
        }));
    }

    if let Some(policy) = policy {
        // Never echo the trust keys themselves — a summary is what an agent can
        // act on, and the keys are large.
        let rules = policy.get("registries").and_then(|r| r.as_array());
        let requiring = rules.map_or(0, |r| {
            r.iter()
                .filter(|x| x.get("require").and_then(|v| v.as_str()) == Some("signed"))
                .count()
        });
        let default_requires = rules
            .and_then(|r| {
                r.iter()
                    .find(|x| x.get("pattern").and_then(|p| p.as_str()) == Some("*"))
            })
            .and_then(|x| x.get("require").and_then(|v| v.as_str()))
            .unwrap_or("none");
        out.insert("signing".into(), json!({
            "defaultRequirement": default_requires,
            "rulesRequiringSignature": requiring,
            "note": "Every image is cosign-verified between pull and start. An unsigned image runs after a logged warning unless a rule requires a signature. Report a refusal; never work around it.",
        }));
    }

    out.insert("secrets".into(), json!({
        "model": "references",
        "note": "A Workload names a registered reference in `secretFrom`; values never appear in a spec, an API response, a log or an event. Register a reference with `cosmonic_secret_set`; the person enters a keychain value in Cosmonic Desktop → Settings → Secrets. A spec naming a reference this host does not have is accepted and parked, not rejected.",
    }));

    Value::Object(out)
}

/// Static descriptors for `resources/list`. `(uri, name, description)`.
pub const RESOURCES: &[(&str, &str, &str)] = &[
    (
        "cosmonic://host",
        "Host status",
        "Daemon health, version, the workload HTTP ingress address, and counts.",
    ),
    (
        "cosmonic://workloads",
        "Workloads",
        "All scheduled workloads with their current status (state, restarts, invocations).",
    ),
    (
        "cosmonic://templates",
        "Project templates",
        "Starter templates for `cosmonic_project_create` (rust-http, go-http, ts-http, rust-mcp, and the rust-/go-nats-<pattern> wasmcloud:nats starters).",
    ),
    (
        "cosmonic://catalog",
        "Catalog",
        "Curated, ready-to-run components you can deploy with `cosmonic_workload_draft`.",
    ),
    (
        "cosmonic://capabilities",
        "Host capabilities",
        "What THIS host can actually run: the embedded wash-runtime, which wasi:http worlds it \
         serves and the exact hostInterfaces binding each needs (the p2/p3 distinction), the \
         NATS driver, whether the loopback door is open, and what the signature policy does on \
         start. Read it before authoring or debugging a Workload.",
    ),
    (
        "cosmonic://schema/workload",
        "Workload schema",
        "The runtime.wasmcloud.dev/v1alpha1 Workload spec shape + a worked HTTP-API example.",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Value {
        json!({
            "version": "0.5.27",
            "runtimeVersion": "2.8.0",
            "os": "macos/aarch64",
            "httpAddr": "127.0.0.1:8200",
            "devBuild": false,
            "messaging": "in-memory (host-wide)",
        })
    }

    #[test]
    fn capabilities_states_both_http_worlds_and_how_each_binds() {
        // The p2/p3 distinction is the most common failure on this host — a
        // component bound to the wrong one starts and never serves — so the
        // resource has to answer it without the model inferring anything.
        let caps = capabilities(Some(&host()), None, None, None);
        let worlds = caps["http"]["worlds"].as_array().expect("worlds");
        assert_eq!(worlds.len(), 2);
        let p3 = &worlds[0];
        assert_eq!(p3["generation"], json!("p3"));
        assert_eq!(p3["hostInterface"]["interfaces"], json!(["handler"]));
        assert_eq!(p3["warmPool"], json!(true));
        let p2 = &worlds[1];
        assert_eq!(p2["generation"], json!("p2"));
        assert_eq!(
            p2["hostInterface"]["interfaces"],
            json!(["incoming-handler"])
        );
        assert_eq!(p2["warmPool"], json!(false));
        // And says the pool knobs are inert there, since accepting-and-ignoring
        // is what makes it look like it worked.
        assert!(p2["note"].as_str().unwrap().contains("IGNORED"));
    }

    #[test]
    fn capabilities_resolves_the_ingress_base_url() {
        let caps = capabilities(Some(&host()), None, None, None);
        assert_eq!(
            caps["host"]["ingressBaseUrl"],
            json!("http://127.0.0.1:8200")
        );
        assert_eq!(caps["host"]["runtimeVersion"], json!("2.8.0"));
    }

    #[test]
    fn a_daemon_that_answers_only_some_routes_still_gets_a_useful_picture() {
        // An older daemon missing a route must not fail the whole read.
        let caps = capabilities(Some(&host()), None, None, None);
        assert!(caps.get("http").is_some());
        assert!(
            caps.get("egress").is_some(),
            "egress defaults must always be stated"
        );
        assert!(caps.get("secrets").is_some());
        assert!(caps.get("nats").is_none());
        assert!(caps.get("signing").is_none());
    }

    #[test]
    fn the_loopback_door_reads_closed_unless_it_is_open() {
        // Fail closed: promising a component can reach a local database when
        // the door is shut sends the user debugging their database.
        let shut = capabilities(Some(&host()), None, Some(&json!({"active": {}})), None);
        assert_eq!(shut["egress"]["hostLoopback"]["enabled"], json!(false));
        assert!(shut["egress"]["hostLoopback"]["note"]
            .as_str()
            .unwrap()
            .contains("CLOSED"));

        let open = capabilities(
            Some(&host()),
            None,
            Some(&json!({ "active": { "allow_host_loopback": true } })),
            None,
        );
        assert_eq!(open["egress"]["hostLoopback"]["enabled"], json!(true));
        assert!(!open["egress"]["hostLoopback"]["note"]
            .as_str()
            .unwrap()
            .contains("CLOSED"));
    }

    #[test]
    fn the_signature_policy_is_summarized_and_never_echoes_a_key() {
        let policy = json!({ "registries": [
            { "pattern": "ghcr.io/cosmonic/*", "require": "signed",
              "keys": ["-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----\n"] },
            { "pattern": "*", "require": "none" },
        ]});
        let caps = capabilities(Some(&host()), Some(&policy), None, None);
        assert_eq!(caps["signing"]["rulesRequiringSignature"], json!(1));
        assert_eq!(caps["signing"]["defaultRequirement"], json!("none"));
        // Keys are large and are not something an agent can act on.
        assert!(
            !caps.to_string().contains("BEGIN PUBLIC KEY"),
            "a trust key reached the resource body"
        );
    }

    #[test]
    fn nats_reports_the_active_settings_and_the_host_owned_connection() {
        let nats = json!({
            "settings": { "servers": "nats://a:4222", "workload_config": "deny" },
            "active": { "servers": "nats://127.0.0.1:4222", "workload_config": "allow" },
        });
        let caps = capabilities(Some(&host()), None, None, Some(&nats));
        // ACTIVE, not settings: a pending change is not what a workload gets.
        assert_eq!(caps["nats"]["servers"], json!("nats://127.0.0.1:4222"));
        assert_eq!(caps["nats"]["workloadConfig"], json!("allow"));
        assert!(caps["nats"]["note"].as_str().unwrap().contains("host owns"));
    }

    /// A `\` line continuation that gets flattened leaves runs of spaces in the
    /// middle of a sentence. It has shipped twice in this crate, and here it
    /// would land in grounding a model reads on every session.
    #[test]
    fn no_capability_text_has_a_collapsed_line_continuation() {
        let caps = capabilities(
            Some(&host()),
            Some(&json!({ "registries": [] })),
            Some(&json!({ "active": {} })),
            Some(&json!({ "active": {} })),
        );
        let mut offenders = Vec::new();
        walk(&caps, &mut offenders);
        assert!(offenders.is_empty(), "double-spaced text: {offenders:?}");

        fn walk(value: &Value, out: &mut Vec<String>) {
            match value {
                Value::String(s) if s.contains("  ") => out.push(s.clone()),
                Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
                Value::Object(o) => o.values().for_each(|v| walk(v, out)),
                _ => {}
            }
        }
    }

    #[test]
    fn every_listed_cosmonic_resource_has_a_mime_type() {
        for (uri, _, description) in RESOURCES {
            assert!(mime_for(uri).contains('/'), "{uri} has no real mime type");
            assert!(
                description.len() > 30,
                "{uri} description is too thin to choose on"
            );
        }
        assert!(RESOURCES
            .iter()
            .any(|(u, _, _)| *u == "cosmonic://capabilities"));
    }
}
