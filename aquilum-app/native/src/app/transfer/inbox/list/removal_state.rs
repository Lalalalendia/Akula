//! A durable removal intent revokes preview/export/import UI eligibility.
//! List/status must agree with InboxLedger::open_verified_file fail-closed checks.
use akula_inbox_ledger_lab::{InboxLedger, LocalStatus, RemovalAuditStatus};
use uuid::Uuid;

pub(super) fn status_key(
    ledger: &InboxLedger,
    id: Uuid,
    local: LocalStatus,
) -> Result<&'static str, String> {
    let removal = ledger
        .removal_status(id)
        .map_err(|error| format!("Inbox removal audit: {error:?}"))?;
    Ok(match removal {
        RemovalAuditStatus::Requested => "transfer.inboxRemovalRequested",
        RemovalAuditStatus::Removed => "transfer.inboxRemoved",
        RemovalAuditStatus::NeedsReview => "transfer.inboxReview",
        RemovalAuditStatus::Available => match local {
            LocalStatus::VerifiedLocal => "transfer.inboxVerified",
            LocalStatus::Pending => "transfer.inboxPending",
            LocalStatus::NeedsReview => "transfer.inboxReview",
            LocalStatus::Unknown => "transfer.inboxUnknown",
        },
    })
}
