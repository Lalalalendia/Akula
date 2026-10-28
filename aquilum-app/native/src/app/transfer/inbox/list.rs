//! Read-only Inbox projection; never trust SQLite without local rehashing.
use super::*;
use crate::ui::transfer::InboxRow;
use akula_inbox_ledger_lab::{ImportAuditStatus, LocalStatus};

pub(super) fn read_checked_inbox(root: &Path) -> Result<InboxView, String> {
    let Some(ledger) =
        InboxLedger::open_existing(root).map_err(|error| format!("read-only ledger: {error:?}"))?
    else {
        return Ok(InboxView::default());
    };
    #[cfg(windows)]
    let _pinned = aquilum_core::p2p_inbox_lab::pin_windows_inbox_read(root)
        .map_err(|error| format!("Windows Inbox read custody refused: {error:?}"))?;
    let total = ledger
        .count()
        .map_err(|error| format!("count inbox: {error:?}"))?;
    let rows = ledger
        .list_recent(24)
        .map_err(|error| format!("check recent received files: {error:?}"))?
        .into_iter()
        .map(|(file, status)| -> Result<InboxRow, String> {
            let imported = ledger.import_status(file.id)
                .map_err(|error| format!("Inbox import audit: {error:?}"))?;
            let id = file.id.hyphenated().to_string();
            let sender: String = file.sender_key[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            Ok(InboxRow {
                id,
                bytes: file.bytes,
                sender,
                status_key: match status {
                    LocalStatus::VerifiedLocal => "transfer.inboxVerified",
                    LocalStatus::Pending => "transfer.inboxPending",
                    LocalStatus::NeedsReview => "transfer.inboxReview",
                    LocalStatus::Unknown => "transfer.inboxUnknown",
                },
                import_audit_key: match imported {
                    ImportAuditStatus::NotRecorded => None,
                    ImportAuditStatus::Verified => Some("transfer.inboxAuditVerified"),
                    ImportAuditStatus::NeedsReview => Some("transfer.inboxAuditReview"),
                },
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(InboxView {
        rows,
        total,
        loading: false,
        error: None,
        exporting: false,
        importing: false,
        previewing: false,
        preview: None,
        notice_key: None,
    })
}

