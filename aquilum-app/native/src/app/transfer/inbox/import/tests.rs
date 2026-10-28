use super::*;
use akula_inbox_ledger_lab::ImportAuditStatus;
use akula_inbox_ledger_lab::{ExpectedTransfer, LocalStatus, Transport};
use aquilum_core::CoreEvent;
use std::{fs, io::Write};

#[test]
fn verified_receipt_import_creates_new_file_only() {
    let temp = tempfile::tempdir().unwrap();
    let appdata = temp.path().join("appdata");
    let inbox = temp.path().join("received-inbox");
    let vault = temp.path().join("vault");
    for path in [&appdata, &vault] {
        fs::create_dir(path).unwrap();
    }
    let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
    core.ui_state
        .resolve_workspace(vault.to_str().unwrap(), 1)
        .unwrap();
    let mut ledger = InboxLedger::open(&inbox).unwrap();
    let bytes = b"verified inbox content";
    let receipt = ExpectedTransfer {
        id: Uuid::new_v4(),
        sender_key: [7; 32],
        recipient_key: [8; 32],
        source: Transport::P2p,
        bytes: bytes.len() as u64,
        blake3: *blake3::hash(bytes).as_bytes(),
    };
    assert_eq!(ledger.reserve(&receipt).unwrap(), LocalStatus::Pending);
    let object = ledger.object_path(receipt.id);
    let mut writer = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&object)
        .unwrap();
    writer.write_all(bytes).unwrap();
    writer.sync_all().unwrap();
    drop(writer);
    assert_eq!(
        ledger.confirm(&receipt).unwrap(),
        LocalStatus::VerifiedLocal
    );
    drop(ledger);
    let imported = import_checked_into_vault(&core, &inbox, receipt.id, &vault).unwrap();
    assert_eq!(fs::read(&imported).unwrap(), bytes);
    assert_eq!(fs::read(&object).unwrap(), bytes);
    assert_eq!(
        read_checked_inbox(&inbox).unwrap().rows[0].import_audit_key,
        Some("transfer.inboxAuditVerified")
    );
    assert!(import_checked_into_vault(&core, &inbox, receipt.id, &vault).is_err());
    assert_eq!(fs::read(&imported).unwrap(), bytes);
    assert_eq!(fs::read(&object).unwrap(), bytes);
    let reader = InboxLedger::open_existing(&inbox).unwrap().unwrap();
    assert_eq!(
        reader.import_status(receipt.id).unwrap(),
        ImportAuditStatus::Verified
    );
    fs::write(&object, b"corrupted").unwrap();
    assert_eq!(
        reader.import_status(receipt.id).unwrap(),
        ImportAuditStatus::NeedsReview
    );
    assert_eq!(
        read_checked_inbox(&inbox).unwrap().rows[0].import_audit_key,
        Some("transfer.inboxAuditReview")
    );
    assert!(import_checked_into_vault(&core, &inbox, receipt.id, &vault).is_err());
}
