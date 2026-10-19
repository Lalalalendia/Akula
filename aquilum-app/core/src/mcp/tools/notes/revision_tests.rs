// Real File Gate conditional edits against a disposable Markdown workspace.
use super::*;
use crate::app_core::{Core, CoreEvent};
use crate::files::gate;
use std::fs;
use std::sync::Arc;

#[test]
fn stale_expected_revision_does_not_rebase_or_overwrite_newer_note() {
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let root = temp.path().join("vault");
    fs::create_dir_all(&appdata).unwrap();
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    let workspace = root.to_string_lossy().into_owned();
    let note = root.join("Shared.md");
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(&workspace, 1).unwrap();
    gate::create(&core, &note, "# Shared\n\noriginal", Source::Agent, None).unwrap();

    let vault = Vault { root, visible: false };
    let first = read_one(&vault, "Shared.md", &json!({ "includeRevision": true }), 8_000).unwrap();
    let original = first["revision"].as_str().unwrap().to_owned();
    assert_eq!(original.len(), 64);

    let changed = update_note(
        &core,
        &json!({
            "note": "Shared.md", "workspace": workspace,
            "mode": "append", "content": "agent A",
            "expectedRevision": original.clone(), "open": false,
        }),
    )
    .unwrap();
    assert_eq!(changed["changed"], true);
    let new_revision = changed["revision"].as_str().unwrap().to_owned();
    assert_ne!(new_revision, original);
    let after_first = fs::read_to_string(&note).unwrap();

    let stale = update_note(
        &core,
        &json!({
            "note": "Shared.md", "workspace": workspace,
            "mode": "replace", "content": "agent B overwrite",
            "expectedRevision": original.clone(), "open": false,
        }),
    )
    .unwrap_err();
    let conflict: Value = serde_json::from_str(&stale).unwrap();
    assert_eq!(conflict["code"], "MCP_REVISION_CONFLICT");
    assert_eq!(conflict["actualRevision"], new_revision);
    assert_eq!(conflict["retryable"], false);
    assert_eq!(fs::read_to_string(&note).unwrap(), after_first);

    let invalid = update_note(
        &core,
        &json!({
            "note": "Shared.md", "workspace": workspace,
            "content": "invalid", "expectedRevision": "not-a-hash",
        }),
    )
    .unwrap_err();
    assert!(invalid.contains("expectedRevision"));
    assert_eq!(fs::read_to_string(&note).unwrap(), after_first);

    let fresh = update_note(
        &core,
        &json!({
            "note": "Shared.md", "workspace": workspace,
            "mode": "append", "content": "agent B",
            "expectedRevision": new_revision, "open": false,
        }),
    )
    .unwrap();
    assert_eq!(fresh["changed"], true);
    assert!(fs::read_to_string(&note).unwrap().contains("agent B"));

    let legacy = read_one(&vault, "Shared.md", &json!({}), 8_000).unwrap();
    assert!(legacy.get("revision").is_none(), "legacy reads stay lightweight");
    core.shutdown();
}

#[test]
fn revision_hash_uses_original_crlf_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let note = root.join("CRLF.md");
    let raw = b"# CRLF\r\n\r\nhello\r\n";
    fs::write(&note, raw).unwrap();
    let vault = Vault { root, visible: false };
    let read = read_one(&vault, "CRLF.md", &json!({ "includeRevision": true }), 8_000).unwrap();
    assert_eq!(read["content"], "# CRLF\n\nhello\n");
    assert_eq!(read["revision"], blake3::hash(raw).to_hex().to_string());
    assert_ne!(
        read["revision"],
        blake3::hash(read["content"].as_str().unwrap().as_bytes()).to_hex().to_string(),
        "raw-byte CAS must not mistake normalized text for disk identity"
    );
}
