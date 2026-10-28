//! Oversize recovery preflight: never enqueue an unpublishable shadow.
use std::fs;
use std::sync::Arc;
use std::time::Duration;

use crop::Rope;
use uuid::Uuid;

use super::SnapshotQueue;

fn digest(path: &std::path::Path) -> String {
    blake3::hash(&fs::read(path).unwrap()).to_hex().to_string()
}

#[test]
fn oversized_shadow_is_rejected_before_revision_or_disk_work() {
    let data = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let doc = vault.path().join("over-limit.md");
    fs::write(&doc, "original").unwrap();
    let workspace = Uuid::new_v4();
    let queue = SnapshotQueue::new(data.path(), vault.path(), workspace, &doc, &digest(&doc)).unwrap();
    let too_large = Rope::from("x".repeat(super::super::MAX_TEXT + 1));

    let error = queue.request(too_large).unwrap_err();
    assert!(error.to_string().contains("16 MiB"));
    let status = queue.status();
    assert!(!status.pending, "over-limit draft must not enter the worker queue");
    assert!(status.durable_revision.is_none(), "no false durability ACK");
    assert_eq!(fs::read_to_string(&doc).unwrap(), "original");
    assert!(super::super::inspect(data.path(), vault.path(), workspace, &doc).unwrap().is_none());

    let revision = queue.request(Rope::from("valid smaller draft")).unwrap();
    crate::disk::wait();
    let status = queue.status();
    assert_eq!(status.durable_revision, Some(revision));
    assert!(status.error.is_none());
    assert_eq!(
        super::super::inspect(data.path(), vault.path(), workspace, &doc).unwrap().unwrap().body,
        "valid smaller draft"
    );
}

#[test]
fn over_limit_recovery_failure_does_not_block_regular_file_gate_save() {
    let data = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let doc = vault.path().join("large-note.md");
    fs::write(&doc, "original").unwrap();
    let workspace = Uuid::new_v4();
    let core = Arc::new(aquilum_core::Core::open(data.path(), Arc::new(|_| {})));
    let (mut note, _) = crate::note::Note::open(doc.clone()).unwrap();
    note.arm_shadow(data.path(), vault.path(), workspace).unwrap();
    let changed = Rope::from("y".repeat(super::super::MAX_TEXT + 1));

    note.edit(Duration::from_secs(30));
    assert!(note.snapshot_unsaved(&changed).is_err(), "over-limit snapshot must fail immediately");
    assert!(note.shadow_status().unwrap().durable_revision.is_none());
    note.flush(&core, &changed);
    crate::disk::wait();
    assert_eq!(fs::metadata(&doc).unwrap().len(), changed.byte_len() as u64);
    assert_eq!(fs::read(&doc).unwrap(), changed.to_string().into_bytes());
    core.shutdown();
}
