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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InboxView {
        InboxView {
            rows: vec![InboxRow {
                id: "00000000-0000-4000-8000-000000000001".to_owned(),
                bytes: 4,
                sender: "sender".to_owned(),
                status_key: "transfer.inboxVerified",
                import_audit_key: None,
            }],
            ..InboxView::default()
        }
    }

    fn rendered_actions(view: &InboxView) -> Vec<InboxAction> {
        let mut actions = Vec::new();
        let row = &view.rows[0];
        let _widget = attach(Flex::row(), row, view, &mut actions);
        actions.into_iter().map(|(_, action)| action).collect()
    }

    #[test]
    fn confirmation_is_separate_from_request_and_blocks_other_rows() {
        let mut view = sample();
        let initial = rendered_actions(&view);
        assert!(initial.iter().any(|action| matches!(action, InboxAction::AskRemoval(_))));
        assert!(!initial.iter().any(|action| matches!(action, InboxAction::ConfirmRemoval(_))));

        view.removal_confirmation = Some(view.rows[0].id.clone());
        let confirmation = rendered_actions(&view);
        assert_eq!(confirmation.len(), 2);
        assert!(confirmation.iter().any(|action| matches!(action, InboxAction::ConfirmRemoval(_))));
        assert!(confirmation.iter().any(|action| matches!(action, InboxAction::CancelRemoval)));
        assert!(!confirmation.iter().any(|action| matches!(action, InboxAction::SaveCopy(_))));
        assert!(!confirmation.iter().any(|action| matches!(action, InboxAction::Import(_))));

        view.removing = true;
        assert!(rendered_actions(&view).is_empty(), "no actions during SQLite request");
        view.removing = false;
        view.removal_confirmation = None;
        view.rows[0].status_key = "transfer.inboxRemovalRequested";
        assert!(rendered_actions(&view).is_empty(), "durable removal request revokes actions");
        view.rows[0].status_key = "transfer.inboxReview";
        assert!(rendered_actions(&view).is_empty(), "corrupt local receipt revokes actions");
    }
}
