//! Opt-in Native one-file sender controls for OS-pinned peers.
//! The only way to create a listener is the explicit user click.

use super::*;

pub(super) fn render(
    view: &NativeSendView,
    contacts: &TransportContactView,
    exactly_one: bool,
) -> (NewWidget<Flex>, Vec<(WidgetId, String)>) {
    let mut actions = Vec::new();
    let mut panel = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.nativeSendTitle")))
        .with_fixed(hint(&t("transfer.nativeSendWarning")));
    if !exactly_one {
        panel = panel.with_fixed(hint(&t("transfer.nativeSendChoose")));
    }
    if contacts.rows.iter().all(|contact| contact.revoked) {
        panel = panel.with_fixed(hint(&t("transfer.nativeSendNoPeers")));
    }
    if view.busy {
        panel = panel.with_fixed(hint(&t("transfer.nativeSendWaiting")));
        if let Some(address) = &view.listening {
            panel = panel.with_fixed(details(address));
        }
    } else {
        for contact in &contacts.rows {
            if contact.revoked || !exactly_one {
                continue;
            }
            let button = NewWidget::new(TextButton::new(
                t("transfer.nativeSendButton"),
                Variant::Ghost,
            ));
            actions.push((button.id(), contact.full_endpoint_id.clone()));
            let short: String = contact.full_endpoint_id.chars().take(16).collect();
            panel = panel.with_fixed(NewWidget::new(
                Flex::row()
                    .with(details(&short), 1.0)
                    .with_fixed(button),
            ));
        }
    }
    if view.success {
        panel = panel.with_fixed(hint(&t("transfer.nativeSendSuccess")));
    }
    if let Some(error) = &view.message {
        panel = panel.with_fixed(hint(&format!(
            "{}: {}", t("transfer.nativeSendFailed"), error
        )));
    }
    let panel = NewWidget::new(panel).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    );
    (panel, actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_send_for_revoked_or_missing_selected_file() {
        let contacts = TransportContactView {
            rows: vec![TransportContactRow {
                full_endpoint_id: "a".repeat(64),
                revoked: true,
            }],
            error: None,
        };
        let (_, actions) = render(&NativeSendView::default(), &contacts, true);
        assert!(actions.is_empty());
        let (_, actions) = render(&NativeSendView::default(), &contacts, false);
        assert!(actions.is_empty());
    }
}
