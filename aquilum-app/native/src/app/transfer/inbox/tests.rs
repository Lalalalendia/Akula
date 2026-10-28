use super::*;
use akula_inbox_ledger_lab::{ExpectedTransfer, Transport};
use std::{fs, io::Write};

#[test]
fn missing_inbox_never_creates_any_path() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("received");
    let model = read_checked_inbox(&missing).unwrap();
    assert_eq!(model.total, 0);
    assert!(model.rows.is_empty());
    assert!(!missing.exists());
}

#[test]
fn native_projection_reports_verified_and_corrupt_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = b"read-only native UI physical receipt";
    let file = ExpectedTransfer {
        id: Uuid::new_v4(),
        sender_key: [11; 32],
        recipient_key: [22; 32],
        source: Transport::P2p,
        bytes: bytes.len() as u64,
        blake3: *blake3::hash(bytes).as_bytes(),
    };
    let inbox_root = dir.path().join("received-inbox");
    let mut ledger = InboxLedger::open(&inbox_root).unwrap();
    assert_eq!(ledger.reserve(&file).unwrap(), LocalStatus::Pending);
    let path = ledger.object_path(file.id);
    let mut created = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap();
    created.write_all(bytes).unwrap();
    created.sync_all().unwrap();
    drop(created);
    assert_eq!(ledger.confirm(&file).unwrap(), LocalStatus::VerifiedLocal);
    drop(ledger);
    let model = read_checked_inbox(&inbox_root).unwrap();
    assert_eq!(model.total, 1);
    assert_eq!(model.rows[0].status_key, "transfer.inboxVerified");
    assert_eq!(
        read_checked_text_preview(&inbox_root, file.id)
            .unwrap()
            .as_deref(),
        Some("read-only native UI physical receipt")
    );

    let export_dir = dir.path().join("external-exports");
    fs::create_dir(&export_dir).unwrap();
    let copy = export_dir.join("new-copy.bin");
    save_checked_copy(&inbox_root, file.id, &copy, &[]).unwrap();
    assert_eq!(fs::read(&copy).unwrap(), bytes);
    // No replacing even a user-created or previously exported file.
    assert!(save_checked_copy(&inbox_root, file.id, &copy, &[]).is_err());
    assert_eq!(fs::read(&copy).unwrap(), bytes);

    fs::write(&path, b"modified").unwrap();
    let model = read_checked_inbox(&inbox_root).unwrap();
    assert_eq!(model.total, 1);
    assert_eq!(model.rows[0].status_key, "transfer.inboxReview");
    assert!(read_checked_text_preview(&inbox_root, file.id).is_err());
    let refused = export_dir.join("never-created.bin");
    assert!(save_checked_copy(&inbox_root, file.id, &refused, &[]).is_err());
    assert!(!refused.exists());
}

#[test]
fn text_preview_fail_closed_for_oversized_and_binary_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let inbox = dir.path().join("received-inbox");
    let mut ledger = InboxLedger::open(&inbox).unwrap();
    for bytes in [vec![b'x'; 4097], vec![b'a', 0, b'b'], vec![0xff, 0xfe]] {
        let intent = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [31; 32],
            recipient_key: [32; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(&bytes).as_bytes(),
        };
        assert_eq!(ledger.reserve(&intent).unwrap(), LocalStatus::Pending);
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(ledger.object_path(intent.id))
            .unwrap();
        output.write_all(&bytes).unwrap();
        output.sync_all().unwrap();
        drop(output);
        assert_eq!(ledger.confirm(&intent).unwrap(), LocalStatus::VerifiedLocal);
        assert!(
            read_checked_text_preview(&inbox, intent.id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            ledger.status(intent.id).unwrap(),
            LocalStatus::VerifiedLocal
        );
    }
    let visible = read_checked_inbox(&inbox).unwrap();
    assert_eq!(visible.total, 3);
    assert!(
        visible
            .rows
            .iter()
            .all(|row| row.status_key == "transfer.inboxVerified")
    );
}

#[test]
fn save_copy_rejects_own_inbox_and_known_vault() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("my-vault");
    fs::create_dir(&vault).unwrap();
    let bytes = b"the user must explicitly leave the protected Vault";
    let intent = ExpectedTransfer {
        id: Uuid::new_v4(),
        sender_key: [51; 32],
        recipient_key: [52; 32],
        source: Transport::P2p,
        bytes: bytes.len() as u64,
        blake3: *blake3::hash(bytes).as_bytes(),
    };
    let inbox = dir.path().join("received-inbox");
    let mut ledger = InboxLedger::open(&inbox).unwrap();
    ledger.reserve(&intent).unwrap();
    let path = ledger.object_path(intent.id);
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    writer.write_all(bytes).unwrap();
    writer.sync_all().unwrap();
    drop(writer);
    ledger.confirm(&intent).unwrap();
    drop(ledger);
    let forbidden_vault = vault.join("unexpected.txt");
    assert!(save_checked_copy(&inbox, intent.id, &forbidden_vault, &[vault.clone()]).is_err());
    assert!(!forbidden_vault.exists());
    let forbidden_inbox = inbox.join("unexpected.txt");
    assert!(save_checked_copy(&inbox, intent.id, &forbidden_inbox, &[]).is_err());
    assert!(!forbidden_inbox.exists());
}
