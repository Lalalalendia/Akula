//! Received Inbox list: real checked on-disk records, display only.
//! No buttons in this preview can launch QUIC, overwrite a destination or
//! promote any received file into the user's Vault.

use super::*;

pub(super) fn render(view: &InboxView) -> NewWidget<Flex> {
    let mut panel = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
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
            let detail = format!("{} · {}", received.sender, t(received.status_key));
            panel = panel.with_fixed(details(&label)).with_fixed(hint(&detail));
            panel = panel.with_fixed(rule());
        }
    }
    NewWidget::new(panel).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    )
}
