//! Verified Inbox UI: explicit local user actions only.
use super::super::*;
mod row_actions;

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
            let entry = Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .with(labels, 1.0);
            let entry = row_actions::attach(entry, received, view, &mut actions);
            panel = panel.with_fixed(
                NewWidget::new(entry)
                    .with_props(PropertySet::new().with(Padding::all(Length::px(size::SPACE_8)))),
            );
            if view.removal_confirmation.as_deref() == Some(received.id.as_str()) {
                panel = panel.with_fixed(hint(&t("transfer.inboxRemoveWarning")));
            }
            #[cfg(all(windows, feature = "p2p-inbox-physical-delete-qa"))]
            if view.physical_removal_confirmation.as_deref() == Some(received.id.as_str()) {
                let digest = received
                    .blake3
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let warning = format!(
                    "{} ID: {} · bytes: {} · BLAKE3: {}",
                    t("transfer.inboxPhysicalWarning"),
                    received.id,
                    received.bytes,
                    digest
                );
                panel = panel.with_fixed(hint(&warning));
            }
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
    if view.removing {
        panel = panel.with_fixed(hint(&t("transfer.inboxRemoveWorking")));
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
