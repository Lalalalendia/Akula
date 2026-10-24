#![cfg(not(feature = "sync-installed-host"))]
//! A synthetic tampered history file is never returned for restore selection.

use aquilum_core::files::gate;
use aquilum_core::history::{store, Source};
use aquilum_core::{Core, CoreEvent, EventSink};
use std::fs;
use std::path::Path;
use std::sync::Arc;

#[test]
fn sealed_history_tamper_is_rejected_without_rewriting_the_primary() {
    let tmp = tempfile::tempdir().unwrap();
    let vault = tmp.path().join("vault");
    let appdata = tmp.path().join("appdata");
    fs::create_dir_all(&vault).unwrap();
    fs::create_dir_all(&appdata).unwrap();
    let events: Arc<dyn EventSink> = Arc::new(|_: CoreEvent| {});
    let core = Core::open(&appdata, events);
    core.ui_state
        .resolve_workspace(vault.to_str().unwrap(), 1_700_000_000_000)
        .unwrap();

    let note = vault.join("History.md");
    let mut current_hash = None;
    let mut b_id = None;
    for (index, value) in ["# A\n", "# B\n", "# C\n"].iter().enumerate() {
        let saved = gate::write(
            &core, &note, value, current_hash.as_deref(), Source::Agent, None,
        ).unwrap();
        current_hash = Some(saved.hash);
        if index == 1 {
            b_id = core.history.page(&|| core.known_roots(), &note, 0, 5)
                .versions.first().map(|v| v.id.clone());
        }
    }
    let b_id = b_id.unwrap();
    assert!(b_id.contains("~b3-"));
    assert_eq!(
        core.history.version(&|| core.known_roots(), &note, Some(&b_id))
            .unwrap().text,
        "# B\n"
    );
    let history = store::history_dir(&vault, Path::new("History.md"));
    fs::write(history.join(&b_id), b"# forged\n").unwrap();
    assert!(core.history.version(&|| core.known_roots(), &note, Some(&b_id)).is_none());
    assert_eq!(fs::read(&note).unwrap(), b"# C\n");
    assert_eq!(core.history.page(&|| core.known_roots(), &note, 0, 5).total, 3);
    core.shutdown();
}
