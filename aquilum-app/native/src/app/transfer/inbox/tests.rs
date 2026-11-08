use super::*;
use akula_inbox_ledger_lab::{ExpectedTransfer, LocalStatus, Transport};
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

#[test]
fn durable_removal_intent_never_displays_verified_inbox_actions_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("received-inbox");
    let bytes = b"verified original must not be presented after remove intent";
    let receipt = ExpectedTransfer {
        id: Uuid::new_v4(),
        sender_key: [61; 32],
        recipient_key: [62; 32],
        source: Transport::P2p,
        bytes: bytes.len() as u64,
        blake3: *blake3::hash(bytes).as_bytes(),
    };
    {
        let mut ledger = InboxLedger::open(&root).unwrap();
        assert_eq!(ledger.reserve(&receipt).unwrap(), LocalStatus::Pending);
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(ledger.object_path(receipt.id))
            .unwrap();
        file.write_all(bytes).unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(ledger.confirm(&receipt).unwrap(), LocalStatus::VerifiedLocal);
    }
    assert_eq!(
        read_checked_inbox(&root).unwrap().rows[0].status_key,
        "transfer.inboxVerified"
    );
    {
        let ledger = InboxLedger::open(&root).unwrap();
        ledger.request_removal(receipt.id).unwrap();
        assert_eq!(fs::read(ledger.object_path(receipt.id)).unwrap(), bytes);
    }
    // The original still exists, but removal consent irrevocably disables
    // preview, Save Copy and Vault import until a separate recovery workflow.
    assert_eq!(
        read_checked_inbox(&root).unwrap().rows[0].status_key,
        "transfer.inboxRemovalRequested"
    );
    {
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert!(cold.open_verified_file(receipt.id).is_err());
    }
    // A crash after physical removal, but before SQLite finish, is review,
    // never a false verified/removed receipt.
    fs::remove_file(
        root.join("objects")
            .join(format!("{}.blob", receipt.id.hyphenated())),
    )
    .unwrap();
    assert_eq!(
        read_checked_inbox(&root).unwrap().rows[0].status_key,
        "transfer.inboxReview"
    );
    // Missing bytes do not prove a checked Windows HANDLE disposition.
    // Native has no witness and must leave the record in review state.
    assert_eq!(
        read_checked_inbox(&root).unwrap().rows[0].status_key,
        "transfer.inboxReview"
    );
}

#[test]
fn request_removal_requires_verified_receipt_and_never_unlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("received-inbox");
    let bytes = b"only mark the receipt, never erase original bytes";
    let expected = ExpectedTransfer {
        id: Uuid::new_v4(),
        sender_key: [71; 32],
        recipient_key: [72; 32],
        source: Transport::P2p,
        bytes: bytes.len() as u64,
        blake3: *blake3::hash(bytes).as_bytes(),
    };
    let mut ledger = InboxLedger::open(&root).unwrap();
    assert_eq!(ledger.reserve(&expected).unwrap(), LocalStatus::Pending);
    assert!(removal::request_checked_removal(&root, expected.id).is_err());
    let object = ledger.object_path(expected.id);
    let mut writer = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&object)
        .unwrap();
    writer.write_all(bytes).unwrap();
    writer.sync_all().unwrap();
    drop(writer);
    assert_eq!(ledger.confirm(&expected).unwrap(), LocalStatus::VerifiedLocal);
    drop(ledger);
    assert!(removal::request_checked_removal(&root, expected.id).is_ok());
    assert_eq!(fs::read(&object).unwrap(), bytes);
    assert_eq!(read_checked_inbox(&root).unwrap().rows[0].status_key,
        "transfer.inboxRemovalRequested");
    assert!(removal::request_checked_removal(&root, expected.id).is_err(),
        "a repeated request is not another authorized operation");
    let reopened = InboxLedger::open_existing(&root).unwrap().unwrap();
    assert!(reopened.open_verified_file(expected.id).is_err());
}

#[test]
fn removal_request_rejects_absent_or_corrupted_inbox_without_creating_paths() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing-inbox");
    assert!(removal::request_checked_removal(&missing, Uuid::new_v4()).is_err());
    assert!(!missing.exists());
    assert!(removal::request_checked_removal(&missing, Uuid::nil()).is_err());
    let root = temp.path().join("received-inbox");
    let bytes = b"original";
    let expected = ExpectedTransfer {
        id: Uuid::new_v4(),
        sender_key: [73; 32],
        recipient_key: [74; 32],
        source: Transport::P2p,
        bytes: bytes.len() as u64,
        blake3: *blake3::hash(bytes).as_bytes(),
    };
    let mut ledger = InboxLedger::open(&root).unwrap();
    ledger.reserve(&expected).unwrap();
    let object = ledger.object_path(expected.id);
    fs::write(&object, bytes).unwrap();
    ledger.confirm(&expected).unwrap();
    drop(ledger);
    fs::write(&object, b"modified").unwrap();
    assert!(removal::request_checked_removal(&root, expected.id).is_err());
    assert_eq!(fs::read(&object).unwrap(), b"modified");
    assert_eq!(read_checked_inbox(&root).unwrap().rows[0].status_key,
        "transfer.inboxReview");
}

#[test]
fn native_inbox_reaches_oldest_receipt_and_recovers_stale_page() {
    let temp = tempfile::tempdir().unwrap();
    let inbox = temp.path().join("received-inbox");
    let mut ledger = InboxLedger::open(&inbox).unwrap();
    let mut ids = Vec::new();
    for n in 0..25 {
        let bytes = format!("verified Native Inbox entry {n}");
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [7; 32],
            recipient_key: [8; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes.as_bytes()).as_bytes(),
        };
        ledger.reserve(&receipt).unwrap();
        let mut out = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(ledger.object_path(receipt.id))
            .unwrap();
        out.write_all(bytes.as_bytes()).unwrap();
        out.sync_all().unwrap();
        drop(out);
        assert_eq!(
            ledger.confirm(&receipt).unwrap(),
            LocalStatus::VerifiedLocal
        );
        ids.push(receipt.id);
    }
    drop(ledger);
    let newest = read_checked_inbox_page(&inbox, 0).unwrap();
    assert_eq!(newest.rows.len(), 24);
    assert_eq!(newest.total, 25);
    assert_eq!(newest.page_offset, 0);
    assert_eq!(newest.rows[0].id, ids[24].to_string());

    let oldest = read_checked_inbox_page(&inbox, 24).unwrap();
    assert_eq!(oldest.rows.len(), 1);
    assert_eq!(oldest.page_offset, 24);
    assert_eq!(oldest.rows[0].id, ids[0].to_string());
    assert_eq!(oldest.rows[0].status_key, "transfer.inboxVerified");

    // A page can become invalid after cleanup; never strand the user there.
    let recovered = read_checked_inbox_page(&inbox, 48).unwrap();
    assert_eq!(recovered.page_offset, 0);
    assert_eq!(recovered.rows[0].id, ids[24].to_string());
}
