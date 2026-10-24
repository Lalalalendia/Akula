#![cfg(not(feature = "sync-installed-host"))]
//! PC-FG-03: exact history IDs and BLAKE3 through the ordinary Core File Gate.
//! Only synthetic workspace files are used; this is not an installed-host or
//! hostile same-UID history authenticity / physical-power-loss test.

use aquilum_core::files::document::hash_bytes;
use aquilum_core::files::gate;
use aquilum_core::history::{store, Source};
use aquilum_core::{Core, CoreEvent, EventSink};
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn events() -> Arc<dyn EventSink> {
    Arc::new(|_: CoreEvent| {})
}

#[test]
fn immutable_agent_versions_keep_id_hash_and_bytes_after_restore_and_cold_open() {
    let scratch = tempfile::tempdir().expect("disposable isolated fixture");
    let vault = scratch.path().join("vault");
    let appdata = scratch.path().join("appdata");
    fs::create_dir_all(&vault).unwrap();
    fs::create_dir_all(&appdata).unwrap();

    let core = Core::open(&appdata, events());
    core.ui_state
        .resolve_workspace(vault.to_str().expect("test path UTF-8"), 1_700_000_000_000)
        .expect("register synthetic Vault");
    assert_eq!(core.known_roots(), vec![fs::canonicalize(&vault).unwrap()]);

    let note = vault.join("Ревизии.md");
    let directory = store::history_dir(&vault, Path::new("Ревизии.md"));
    let variants = ["# A\nСтарое 🦈\n", "# B\nИзменение 🦈\n", "# C\nТретья версия 🦈\n"];
    let mut expected_hash: Option<String> = None;
    let mut fixed = Vec::new();

    // Agent revisions, unlike an active Source::Me typing milestone, are
    // separate committed history files and must never coalesce in place.
    for (index, content) in variants.iter().enumerate() {
        let saved = gate::write(
            &core, &note, content, expected_hash.as_deref(), Source::Agent, None,
        )
        .expect("ordinary File Gate save");
        assert_eq!(saved.hash, hash_bytes(content.as_bytes()));
        expected_hash = Some(saved.hash);

        let page = core.history.page(&|| core.known_roots(), &note, 0, 20);
        assert_eq!(page.total, index + 1, "one new agent revision per save");
        let latest = page.versions.first().expect("latest history version");
        assert_eq!(latest.source, Source::Agent);
        let id = latest.id.clone();
        assert!(fixed.iter().all(|(prior, _, _): &(String, Vec<u8>, u64)| prior != &id));
        let exact = fs::read(directory.join(&id)).expect("exact version bytes");
        assert_eq!(exact, content.as_bytes());
        assert_eq!(hash_bytes(&exact), hash_bytes(content.as_bytes()));
        assert_eq!(
            core.history.version(&|| core.known_roots(), &note, Some(&id))
                .expect("read by version ID").text,
            *content
        );
        fixed.push((id, exact, latest.at_ms));
    }

    let (_, original_b, b_at_ms) = &fixed[1];
    let b_text = std::str::from_utf8(original_b).unwrap();
    assert_eq!(fs::read(&note).unwrap(), variants[2].as_bytes());
    assert!(core.history.version(&|| core.known_roots(), &note, Some("missing-id")).is_none());

    let stale = "0".repeat(64);
    assert_ne!(Some(stale.as_str()), expected_hash.as_deref());
    assert!(
        gate::write(&core, &note, b_text, Some(&stale), Source::Restore(*b_at_ms), None)
            .is_err(),
        "stale expected hash must not change the primary or history"
    );
    assert_eq!(fs::read(&note).unwrap(), variants[2].as_bytes());
    assert_eq!(core.history.page(&|| core.known_roots(), &note, 0, 20).total, 3);

    // A legitimate restore publishes a *new* version; it cannot rewrite B's
    // existing ID or any of the original A/B/C bytes and BLAKE3 digests.
    let restored = gate::write(
        &core, &note, b_text, expected_hash.as_deref(), Source::Restore(*b_at_ms), None,
    )
    .expect("restore B through File Gate");
    assert_eq!(restored.hash, hash_bytes(original_b));
    assert_eq!(fs::read(&note).unwrap(), original_b.as_slice());
    let page = core.history.page(&|| core.known_roots(), &note, 0, 20);
    assert_eq!(page.total, 4);
    assert_eq!(page.versions[0].source, Source::Restore(*b_at_ms));
    assert_eq!(page.versions[0].from_ms, Some(*b_at_ms));
    assert!(fixed.iter().all(|(old_id, _, _)| *old_id != page.versions[0].id));

    for (id, bytes, _) in &fixed {
        let current_bytes = fs::read(directory.join(id)).expect("original version remains");
        assert_eq!(current_bytes, *bytes, "history ID must retain exact bytes");
        assert_eq!(hash_bytes(&current_bytes), hash_bytes(bytes));
    }

    // Repeating an identical restore must not silently create a fifth ID.
    gate::write(
        &core, &note, b_text, Some(&restored.hash), Source::Restore(*b_at_ms), None,
    )
    .expect("identical repeated restore");
    assert_eq!(core.history.page(&|| core.known_roots(), &note, 0, 20).total, 4);

    core.shutdown();
    drop(core);
    let cold = Core::open(&appdata, events());
    assert_eq!(cold.known_roots(), vec![fs::canonicalize(&vault).unwrap()]);
    assert_eq!(cold.history.page(&|| cold.known_roots(), &note, 0, 20).total, 4);
    for (id, bytes, _) in &fixed {
        assert_eq!(
            cold.history.version(&|| cold.known_roots(), &note, Some(id))
                .expect("stable ID after Core restart").text.as_bytes(),
            bytes.as_slice()
        );
        assert_eq!(hash_bytes(&fs::read(directory.join(id)).unwrap()), hash_bytes(bytes));
    }
    assert_eq!(fs::read(&note).unwrap(), original_b.as_slice());
    cold.shutdown();
}
