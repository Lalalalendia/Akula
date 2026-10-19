//! D0: Core startup + safe shutdown without any Masonry/winit window host.
//! This is NOT a server/API. The only fixture write is initial setup.
//! Core gets a DIFFERENT writable appdata location for its own state.

use aquilum_core::files::document::read_file_snapshot_impl;
use aquilum_core::{Core, CoreEvent, EventSink};
use std::{fs, sync::Arc};

#[test]
fn headless_core_opens_and_restarts_without_mutating_fixture_vault() {
    let scratch = tempfile::tempdir().expect("isolated fixture");
    let vault = scratch.path().join("vault");
    let appdata = scratch.path().join("appdata");
    fs::create_dir(&vault).unwrap();
    fs::create_dir(&appdata).unwrap();

    let note = vault.join("note.md");
    let original = "Akula headless smoke: UTF-8 and **Markdown**.\n".as_bytes().to_vec();
    fs::write(&note, &original).unwrap();
    let hash = blake3::hash(&original).to_hex().to_string();

    for _ in 0..2 {
        let events: Arc<dyn EventSink> = Arc::new(|_event: CoreEvent| {});
        let core = Core::open(&appdata, events);

        // Core::open does not start MCP by itself. MCP has mutating tools
        // and MUST NOT be exposed as a headless read-only API.
        assert!(!core.mcp.status().running);
        assert!(core.known_roots().is_empty());

        // Fixed local test path only, never untrusted network input.
        // D1 must separately pin roots, reject symlinks/races, and bound reads.
        let snapshot = read_file_snapshot_impl(&note).unwrap();
        assert_eq!(snapshot.content.as_bytes(), original);
        assert_eq!(snapshot.hash, hash);

        core.shutdown();
        drop(core);
        assert_eq!(fs::read(&note).unwrap(), original);
        assert_eq!(fs::read_dir(&vault).unwrap().count(), 1);
    }
}
