use super::{Capability, ManifestError, PluginManifest};
use serde_json::{Value, json};

const VALID: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/valid-v1.json"
));
const FUTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/invalid-future-schema.json"
));
const UNKNOWN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/invalid-unknown-permission.json"
));
const TRAVERSAL: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/plugins/fixtures/invalid-entry-traversal.json"
));

fn changed(f: impl FnOnce(&mut Value)) -> Result<PluginManifest, ManifestError> {
    let mut v: Value = serde_json::from_str(VALID).expect("fixture JSON is valid");
    f(&mut v);
    PluginManifest::parse_json(&v.to_string())
}

#[test]
fn valid_golden_manifest_roundtrips_without_granting_authority() {
    let manifest = PluginManifest::parse_json(VALID).expect("valid fixture");
    assert_eq!(manifest.id, "org.example.word-count");
    assert_eq!(manifest.permissions.required, vec![Capability::UiCommand]);
    assert_eq!(manifest.permissions.optional, vec![Capability::NotesRead]);
    let encoded = manifest.to_json_pretty().unwrap();
    assert_eq!(PluginManifest::parse_json(&encoded).unwrap(), manifest);
}

#[test]
fn unsupported_manifest_versions_fail_closed() {
    assert_eq!(
        PluginManifest::parse_json(FUTURE).unwrap_err(),
        ManifestError::Invalid("unsupported manifest schema")
    );
    assert_eq!(
        changed(|v| v["api"]["min"] = json!(2)).unwrap_err(),
        ManifestError::Invalid("incompatible plugin API range")
    );
    assert_eq!(
        changed(|v| v["api"]["max"] = json!(0)).unwrap_err(),
        ManifestError::Invalid("incompatible plugin API range")
    );
}

#[test]
fn unknown_capabilities_and_keys_are_rejected() {
    assert!(matches!(
        PluginManifest::parse_json(UNKNOWN),
        Err(ManifestError::InvalidJson(_))
    ));
    assert!(matches!(
        changed(|v| v["undocumented_host_grant"] = json!(true)),
        Err(ManifestError::InvalidJson(_))
    ));
    assert!(matches!(
        changed(|v| v["permissions"]["silent_exec"] = json!(true)),
        Err(ManifestError::InvalidJson(_))
    ));
}

#[test]
fn manifest_rejects_path_traversal_and_os_paths() {
    assert_eq!(
        PluginManifest::parse_json(TRAVERSAL).unwrap_err(),
        ManifestError::Invalid("invalid WASM package entry")
    );
    for path in ["/plugin.wasm", "C:\\plugin.wasm", "plugin\\main.wasm", "a//b.wasm", "a/./b.wasm"] {
        assert_eq!(
            changed(|v| v["entry"] = json!(path)).unwrap_err(),
            ManifestError::Invalid("invalid WASM package entry")
        );
    }
}

#[test]
fn plugin_namespace_must_match_claimed_publisher() {
    assert_eq!(
        changed(|v| v["publisher"] = json!("org.attacker")).unwrap_err(),
        ManifestError::Invalid("plugin ID is outside publisher namespace")
    );
    assert_eq!(
        changed(|v| v["id"] = json!("evil")).unwrap_err(),
        ManifestError::Invalid("invalid reverse-DNS identity")
    );
    assert_eq!(
        changed(|v| v["id"] = json!("org.example.Bad")).unwrap_err(),
        ManifestError::Invalid("invalid reverse-DNS identity")
    );
}

#[test]
fn duplicate_platforms_permissions_and_dependencies_are_rejected() {
    assert_eq!(
        changed(|v| v["platforms"] = json!(["linux", "linux"])).unwrap_err(),
        ManifestError::Invalid("duplicate platform")
    );
    assert_eq!(
        changed(|v| v["permissions"]["required"] = json!(["ui.command", "ui.command"]))
            .unwrap_err(),
        ManifestError::Invalid("duplicate or overlapping capability")
    );
    assert_eq!(
        changed(|v| v["permissions"]["optional"] = json!(["ui.command"]))
            .unwrap_err(),
        ManifestError::Invalid("duplicate or overlapping capability")
    );
    assert_eq!(
        changed(|v| v["dependencies"] = json!([
            {"id":"org.other.calendar","min_version":"1.0.0"},
            {"id":"org.other.calendar","min_version":"1.0.0"}
        ])).unwrap_err(),
        ManifestError::Invalid("duplicate dependency")
    );
}

#[test]
fn self_dependency_and_invalid_versions_are_rejected() {
    assert_eq!(
        changed(|v| v["dependencies"] = json!([
            {"id":"org.example.word-count","min_version":"1.0.0"}
        ])).unwrap_err(),
        ManifestError::Invalid("invalid plugin dependency")
    );
    for release in ["01.2.3", "1.2", "1.2.3-beta", "999999999999.0.0"] {
        assert_eq!(
            changed(|v| v["version"] = json!(release)).unwrap_err(),
            ManifestError::Invalid("invalid release version")
        );
    }
}

#[test]
fn network_grants_require_explicit_nonlocal_domains() {
    assert_eq!(
        changed(|v| v["permissions"]["required"] = json!(["network.fetch"]))
            .unwrap_err(),
        ManifestError::Invalid("network.fetch must declare one or more restricted domains")
    );
    assert_eq!(
        changed(|v| v["network_domains"] = json!(["api.example.org"])).unwrap_err(),
        ManifestError::Invalid("network.fetch must declare one or more restricted domains")
    );
    let granted = changed(|v| {
        v["permissions"]["optional"] = json!(["notes.read", "network.fetch"]);
        v["network_domains"] = json!(["api.example.org"]);
    })
    .unwrap();
    assert_eq!(granted.network_domains, vec!["api.example.org"]);
    for host in ["localhost", "127.0.0.1", "metadata.internal", "*.example.org", "a..org", "EXAMPLE.org"] {
        assert_eq!(
            changed(|v| {
                v["permissions"]["optional"] = json!(["network.fetch"]);
                v["network_domains"] = json!([host]);
            }).unwrap_err(),
            ManifestError::Invalid("invalid or duplicate network domain"),
        );
    }
}

#[test]
fn manifest_size_is_bounded_before_deserialization() {
    assert_eq!(
        PluginManifest::parse_json(&" ".repeat(64 * 1024 + 1)).unwrap_err(),
        ManifestError::Invalid("manifest exceeds 64 KiB")
    );
}

#[test]
fn source_does_not_treat_a_manifest_as_a_grant() {
    let manifest = PluginManifest::parse_json(VALID).unwrap();
    assert!(manifest.network_domains.is_empty());
    assert!(manifest.dependencies.is_empty());
    assert!(!manifest.permissions.required.contains(&Capability::NotesRead));
    // The manifest API deliberately has no invoke/install/run/authorize method.
}
