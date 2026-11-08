//! Read-only Inbox projection; never trust SQLite without local rehashing.
use super::*;
use crate::ui::transfer::InboxRow;
use akula_inbox_ledger_lab::ImportAuditStatus;
use super::kind::checked_kind;
mod removal_state;
pub(super) const PAGE_SIZE: usize = 24;
use removal_state::status_key;

#[cfg(test)]
pub(super) fn read_checked_inbox(root: &Path) -> Result<InboxView, String> {
    read_checked_inbox_page(root, 0)
}

pub(super) fn read_checked_inbox_page(root: &Path, offset: usize) -> Result<InboxView, String> {
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
    // An old page may become empty after local cleanup; recover the first page.
    let offset = if u64::try_from(offset).map_or(true, |value| value >= total) {
        0
    } else {
        offset
    };
    let rows = ledger
        .list_page(PAGE_SIZE, offset)
        .map_err(|error| format!("check recent received files: {error:?}"))?
        .into_iter()
        .map(|(file, status)| -> Result<InboxRow, String> {
            let imported = ledger
                .import_status(file.id)
                .map_err(|error| format!("Inbox import audit: {error:?}"))?;
            let id = file.id.hyphenated().to_string();
            let sender: String = file.sender_key[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let mut checked_status = status_key(&ledger, file.id, status)?;
            // Never inspect bytes for pending, revoked or removal-requested
            // entries. Type inspection has no file or Vault authority.
            let kind_key = if checked_status == "transfer.inboxVerified" {
                match checked_kind(&ledger, file.id) {
                    Ok(kind) => kind,
                    Err(_) => {
                        // A second same-handle verification can detect a
                        // race after list_page(); never keep Verified on error.
                        checked_status = "transfer.inboxReview";
                        "transfer.inboxKindUnknown"
                    }
                }
            } else {
                "transfer.inboxKindUnknown"
            };
            Ok(InboxRow {
                id,
                bytes: file.bytes,
                sender,
                kind_key,
                status_key: checked_status,
                #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
                blake3: file.blake3,
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
        page_offset: offset,
        ..InboxView::default()
    })
}
