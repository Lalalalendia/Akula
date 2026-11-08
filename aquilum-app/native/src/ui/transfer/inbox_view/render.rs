//! Verified Inbox UI: explicit local user actions only.
use super::super::*;

pub(in crate::ui::transfer) fn render(
    view: &InboxView,
) -> (NewWidget<Flex>, Vec<(WidgetId, InboxAction)>) {
    let mut panel = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
    let mut actions = Vec::new();
    if view.loading {
        panel = panel.with_fixed(hint(&t("transfer.inboxLoading")));
    } else if let Some(_error) = &view.error {
        panel = panel.with_fixed(hint(&t("transfer.inboxUnavailable")));
    } else if view.rows.is_empty() {
        panel = panel.with_fixed(hint(&t("transfer.inboxEmpty")));
    } else {
        panel = panel.with_fixed(hint(&t_with(
            "transfer.inboxSummary",
            &[("count", &view.total.to_string())],
        )));
        for received in &view.rows {
            let label = format!("{} · {}", received.id, display_size(received.bytes));
            let detail = match received.import_audit_key {
                Some(key) => format!(
                    "{} · {} · {}",
                    received.sender,
                    t(received.status_key),
                    t(key)
                ),
                None => format!("{} · {}", received.sender, t(received.status_key)),
            };
            let labels = NewWidget::new(
                Flex::column()
                    .cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_fixed(details(&label))
                    .with_fixed(hint(&detail)),
            );
            let mut entry = Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .with(labels, 1.0);
            if received.status_key == "transfer.inboxVerified"
                && !view.exporting
                && !view.importing
                && !view.previewing
            {
                let preview =
                    NewWidget::new(TextButton::new(t("transfer.inboxPreview"), Variant::Ghost));
                actions.push((preview.id(), InboxAction::Preview(received.id.clone())));
                entry = entry.with_fixed(preview);
                let save =
                    NewWidget::new(TextButton::new(t("transfer.inboxSaveCopy"), Variant::Ghost));
                actions.push((save.id(), InboxAction::SaveCopy(received.id.clone())));
                entry = entry.with_fixed(save);
                #[cfg(windows)]
                {
                    if received.import_audit_key.is_none() {
                        let import = NewWidget::new(TextButton::new(
                            t("transfer.inboxImport"),
                            Variant::Ghost,
                        ));
                        actions.push((import.id(), InboxAction::Import(received.id.clone())));
                        entry = entry.with_fixed(import);
                    }
                }
            }
            panel = panel.with_fixed(
                NewWidget::new(entry)
                    .with_props(PropertySet::new().with(Padding::all(Length::px(size::SPACE_8)))),
            );
            if let Some((id, text)) = &view.preview {
                if id == &received.id && received.status_key == "transfer.inboxVerified" {
                    panel = panel.with_fixed(hint(text));
                }
            }
            panel = panel.with_fixed(rule());
        }
    }
    if view.exporting {
        panel = panel.with_fixed(hint(&t("transfer.inboxSaving")));
    }
    #[cfg(windows)]
    if view.importing {
        panel = panel.with_fixed(hint(&t("transfer.inboxImporting")));
    }
    if view.previewing {
        panel = panel.with_fixed(hint(&t("transfer.inboxPreviewWorking")));
    }
    if let Some(key) = view.notice_key {
        panel = panel.with_fixed(hint(&t(key)));
    }
    let panel = NewWidget::new(panel).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    );
    (panel, actions)
}
