//! Render local draft queue only; actions cannot start networking.
use super::*;

pub(super) fn render(rows: &[QueueRow]) -> (NewWidget<Flex>, Vec<(WidgetId, usize, QueueAction)>) {
    let mut actions = Vec::new();
    let mut queue = Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch);
    if rows.is_empty() {
        queue = queue.with_fixed(hint(&t("transfer.empty")));
    }
    for row in rows {
        let state = if row.paused {
            t("transfer.paused")
        } else {
            t("transfer.pending")
        };
        let summary = t_with(
            "transfer.summary",
            &[("size", &display_size(row.bytes)), ("state", &state)],
        );
        let labels = NewWidget::new(
            Flex::column()
                .cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_fixed(details(&row.name))
                .with_fixed(hint(&summary)),
        );
        let toggle = NewWidget::new(TextButton::new(
            if row.paused {
                t("transfer.resume")
            } else {
                t("transfer.pause")
            },
            Variant::Ghost,
        ));
        let remove = NewWidget::new(TextButton::new(t("transfer.remove"), Variant::Ghost));
        actions.push((toggle.id(), row.index, QueueAction::Toggle));
        actions.push((remove.id(), row.index, QueueAction::Remove));
        let buttons = NewWidget::new(Flex::row().with_fixed(toggle).with_fixed(remove));
        let entry = NewWidget::new(
            Flex::row()
                .cross_axis_alignment(CrossAxisAlignment::Center)
                .with(labels, 1.0)
                .with_fixed(buttons),
        )
        .with_props(
            PropertySet::new()
                .with(Gap::new(Length::px(size::SPACE_8)))
                .with(Padding::all(Length::px(size::SPACE_8))),
        );
        queue = queue.with_fixed(entry);
        queue = queue.with_fixed(rule());
    }
    let queue = NewWidget::new(queue).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    );

    (queue, actions)
}
