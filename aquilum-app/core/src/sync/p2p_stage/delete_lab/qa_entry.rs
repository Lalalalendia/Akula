//! Explicit non-shipping Core QA delete entrypoint.
//! Caller independently binds the selected UUID, bytes and digest to local consent.
//! This is not an installed UI action, a remote command or a proof of power-loss durability.
use super::{
    delete_after_explicit_consent, ConfirmedRemoval, CrashCut, InboxLedger, Path, Uuid, MAX_BYTES,
};

pub(crate) fn qa_dispose_confirmed_inbox_file(
    root: &Path,
    selected_id: Uuid,
    confirmed_bytes: u64,
    confirmed_hash: [u8; 32],
) -> Result<(), String> {
    if selected_id.is_nil()
        || !(1..=MAX_BYTES).contains(&confirmed_bytes)
        || confirmed_hash == [0; 32]
    {
        return Err("invalid independent confirmation".into());
    }
    // Pin an existing root and database before opening a writable ledger.
    // A missing path must never create an Inbox as a side effect.
    let _custody = super::super::pin_p2p_read_snapshot(root)
        .map_err(|error| format!("QA root custody rejected: {error:?}"))?;
    let observed = InboxLedger::open_existing(root)
        .map_err(|error| format!("QA ledger read refused: {error:?}"))?
        .ok_or("QA Inbox ledger missing")?;
    let (verified, receipt) = observed
        .open_verified_file(selected_id)
        .map_err(|error| format!("QA selected receipt rejected: {error:?}"))?;
    if receipt.bytes != confirmed_bytes || receipt.blake3 != confirmed_hash {
        return Err("QA confirmation differs from selected verified receipt".into());
    }
    drop(verified);
    drop(observed);
    let ledger =
        InboxLedger::open(root).map_err(|error| format!("QA ledger write refused: {error:?}"))?;
    delete_after_explicit_consent(
        &ledger,
        root,
        selected_id,
        Some(ConfirmedRemoval {
            transfer_id: selected_id,
            bytes: confirmed_bytes,
            blake3: confirmed_hash,
        }),
        CrashCut::None,
        || {},
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use akula_inbox_ledger_lab::{ExpectedTransfer, LocalStatus, RemovalAuditStatus, Transport};
    use std::fs;

    #[test]
    fn opt_in_qa_entry_requires_bound_confirmation_and_no_retry() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("inbox");
        let bytes = b"QA independently selected receipt";
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [41; 32],
            recipient_key: [42; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(&root).unwrap();
        ledger.reserve(&receipt).unwrap();
        let path = ledger.object_path(receipt.id);
        fs::write(&path, bytes).unwrap();
        assert_eq!(
            ledger.confirm(&receipt).unwrap(),
            LocalStatus::VerifiedLocal
        );
        let unrelated = root.join("unrelated.txt");
        fs::write(&unrelated, b"preserve").unwrap();
        drop(ledger);
        let wrong_id = Uuid::new_v4();
        assert!(
            qa_dispose_confirmed_inbox_file(&root, Uuid::nil(), receipt.bytes, receipt.blake3)
                .is_err()
        );
        assert!(
            qa_dispose_confirmed_inbox_file(&root, wrong_id, receipt.bytes, receipt.blake3)
                .is_err()
        );
        assert!(
            qa_dispose_confirmed_inbox_file(&root, receipt.id, receipt.bytes, [77; 32]).is_err()
        );
        assert!(qa_dispose_confirmed_inbox_file(
            &root,
            receipt.id,
            receipt.bytes + 1,
            receipt.blake3
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            InboxLedger::open_existing(&root)
                .unwrap()
                .unwrap()
                .removal_status(receipt.id)
                .unwrap(),
            RemovalAuditStatus::Available
        );
        qa_dispose_confirmed_inbox_file(&root, receipt.id, receipt.bytes, receipt.blake3).unwrap();
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(
            cold.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::Removed
        );
        assert!(!path.exists());
        assert!(
            qa_dispose_confirmed_inbox_file(&root, receipt.id, receipt.bytes, receipt.blake3)
                .is_err()
        );
        assert_eq!(fs::read(&unrelated).unwrap(), b"preserve");
    }


    #[test]
    fn earlier_requested_intent_never_implicitly_reauthorizes_physical_deletion() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("inbox");
        let bytes = b"preserve bytes after earlier user request";
        let receipt = ExpectedTransfer {
            id: Uuid::new_v4(),
            sender_key: [51; 32],
            recipient_key: [52; 32],
            source: Transport::P2p,
            bytes: bytes.len() as u64,
            blake3: *blake3::hash(bytes).as_bytes(),
        };
        let mut ledger = InboxLedger::open(&root).unwrap();
        ledger.reserve(&receipt).unwrap();
        let path = ledger.object_path(receipt.id);
        fs::write(&path, bytes).unwrap();
        assert_eq!(ledger.confirm(&receipt).unwrap(), LocalStatus::VerifiedLocal);
        ledger.request_removal(receipt.id).unwrap();
        drop(ledger);

        assert!(qa_dispose_confirmed_inbox_file(
            &root,
            receipt.id,
            receipt.bytes,
            receipt.blake3,
        )
        .is_err());
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(
            cold.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::Requested
        );
        assert_eq!(fs::read(&path).unwrap(), bytes);
        drop(cold);

        // A vanished pathname is not a disposition witness and cannot
        // authorize a retry or a false "Removed" journal state.
        fs::remove_file(&path).unwrap();
        assert!(qa_dispose_confirmed_inbox_file(
            &root,
            receipt.id,
            receipt.bytes,
            receipt.blake3,
        )
        .is_err());
        let cold = InboxLedger::open_existing(&root).unwrap().unwrap();
        assert_eq!(
            cold.removal_status(receipt.id).unwrap(),
            RemovalAuditStatus::NeedsReview
        );
    }

    #[test]
    fn qa_entry_refuses_missing_root_without_creating_it() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("missing-inbox");
        assert!(qa_dispose_confirmed_inbox_file(
            &root,
            Uuid::new_v4(),
            4,
            *blake3::hash(b"data").as_bytes(),
        )
        .is_err());
        assert!(!root.exists());
    }
}
