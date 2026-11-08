//! Verified-only actions; removal request is a two-click QA intent, not file unlink.
use super::*;

pub(super) fn attach(
    mut entry: Flex,
    received: &InboxRow,
    view: &InboxView,
    actions: &mut Vec<(WidgetId, InboxAction)>,
) -> Flex {
    if received.status_key != "transfer.inboxVerified"
        || view.exporting
        || view.importing
        || view.previewing
        || view.removing
    {
        return entry;
    }
    if view.removal_confirmation.as_deref() == Some(received.id.as_str()) {
        let confirm = NewWidget::new(TextButton::new(
            t("transfer.inboxRemoveConfirm"),
            Variant::Ghost,
        ));
        actions.push((confirm.id(), InboxAction::ConfirmRemoval(received.id.clone())));
        entry = entry.with_fixed(confirm);
        let cancel = NewWidget::new(TextButton::new(
            t("transfer.inboxRemoveCancel"),
            Variant::Ghost,
        ));
        actions.push((cancel.id(), InboxAction::CancelRemoval));
        return entry.with_fixed(cancel);
    }
    if view.removal_confirmation.is_some() {
        return entry;
    }
    let preview = NewWidget::new(TextButton::new(t("transfer.inboxPreview"), Variant::Ghost));
    actions.push((preview.id(), InboxAction::Preview(received.id.clone())));
    entry = entry.with_fixed(preview);
    let save = NewWidget::new(TextButton::new(t("transfer.inboxSaveCopy"), Variant::Ghost));
    actions.push((save.id(), InboxAction::SaveCopy(received.id.clone())));
    entry = entry.with_fixed(save);
    #[cfg(windows)]
    if received.import_audit_key.is_none() {
        let import = NewWidget::new(TextButton::new(t("transfer.inboxImport"), Variant::Ghost));
        actions.push((import.id(), InboxAction::Import(received.id.clone())));
        entry = entry.with_fixed(import);
    }
    let removal = NewWidget::new(TextButton::new(
        t("transfer.inboxRemoveIntent"),
        Variant::Ghost,
    ));
    actions.push((removal.id(), InboxAction::AskRemoval(received.id.clone())));
    entry.with_fixed(removal)
}
