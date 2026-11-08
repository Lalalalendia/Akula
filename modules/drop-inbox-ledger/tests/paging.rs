//! Post-24 Inbox access must remain read-only and independently verified.
use akula_inbox_ledger_lab::{ExpectedTransfer, InboxLedger, LocalStatus, Transport};
use std::fs;
use uuid::Uuid;

#[test]
fn older_pages_remain_selectable_after_more_than_24_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("inbox");
    let mut ledger = InboxLedger::open(&root).unwrap();
    let mut inserted = Vec::new();
    for n in 0..27 {
        let bytes = format!("independent verified local Inbox receipt {n}");
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [7; 32],
            recipient_key: [9; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes.as_bytes()).as_bytes(),
        };
        ledger.reserve(&receipt).unwrap();
        fs::write(ledger.object_path(receipt.id), bytes.as_bytes()).unwrap();
        assert_eq!(
            ledger.confirm(&receipt).unwrap(),
            LocalStatus::VerifiedLocal
        );
        inserted.push(receipt.id);
    }
    assert_eq!(ledger.count().unwrap(), 27);
    let newest = ledger.list_recent(24).unwrap();
    assert_eq!(newest, ledger.list_page(24, 0).unwrap());
    assert_eq!(newest.len(), 24);
    assert_eq!(newest[0].0.id, inserted[26]);
    assert_eq!(newest[23].0.id, inserted[3]);

    let older = ledger.list_page(24, 24).unwrap();
    assert_eq!(older.len(), 3);
    assert_eq!(older[0].0.id, inserted[2]);
    assert_eq!(older[1].0.id, inserted[1]);
    assert_eq!(older[2].0.id, inserted[0]);
    assert!(older
        .iter()
        .all(|(_, status)| *status == LocalStatus::VerifiedLocal));
    assert!(ledger.list_page(24, 27).unwrap().is_empty());

    // A stale object on an older page cannot retain a false "Verified" label.
    fs::write(
        ledger.object_path(inserted[0]),
        b"changed after confirmation",
    )
    .unwrap();
    let page = ledger.list_page(24, 24).unwrap();
    assert_eq!(page[2].0.id, inserted[0]);
    assert_eq!(page[2].1, LocalStatus::NeedsReview);
    assert_eq!(ledger.list_recent(24).unwrap().len(), 24);
    assert!(ledger.list_page(0, 0).is_err());
    assert!(ledger.list_page(65, 0).is_err());
    if usize::BITS > 63 {
        assert!(ledger.list_page(24, usize::MAX).is_err());
    }
}
