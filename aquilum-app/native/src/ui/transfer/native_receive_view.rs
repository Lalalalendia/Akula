//! Native one-file receiver actions only for OS-pinned contacts.
//! Clipboard contains an address, never peer identity or filesystem path.

use super::*;

pub(super) fn render(
    view: &NativeReceiveView,
    contacts: &TransportContactView,
) -> (NewWidget<Flex>, Vec<(WidgetId, NativeReceiveAction)>) {
    let mut actions = Vec::new();
    let mut panel = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.nativeReceiveTitle")))
        .with_fixed(hint(&t("transfer.nativeReceiveInstruction")))
        .with_fixed(hint(&t("transfer.nativeInternetDisclosure")));
    if contacts.rows.iter().all(|contact| contact.revoked) {
        panel = panel.with_fixed(hint(&t("transfer.nativeReceiveNone")));
    }
    if view.busy {
        panel = panel.with_fixed(hint(&t("transfer.nativeReceiveWorking")));
        if !view.canceling {
            let cancel = NewWidget::new(TextButton::new(
                t("transfer.nativeReceiveCancel"),
                Variant::Ghost,
            ));
            actions.push((cancel.id(), NativeReceiveAction::Cancel));
            panel = panel.with_fixed(cancel);
        }
    } else {
        for contact in contacts
            .rows
            .iter()
            .filter(|contact| !contact.revoked)
            .take(16)
        {
            let button = NewWidget::new(TextButton::new(
                t("transfer.nativeReceiveButton"),
                Variant::Ghost,
            ));
            actions.push((
                button.id(),
                NativeReceiveAction::Start(contact.full_endpoint_id.clone()),
            ));
            let internet = NewWidget::new(TextButton::new(
                t("transfer.nativeReceiveInternetButton"),
                Variant::Ghost,
            ));
            actions.push((
                internet.id(),
                NativeReceiveAction::StartInternet(contact.full_endpoint_id.clone()),
            ));
            let short: String = contact.full_endpoint_id.chars().take(16).collect();
            panel = panel.with_fixed(NewWidget::new(
                Flex::row()
                    .with(details(&short), 1.0)
                    .with_fixed(button)
                    .with_fixed(internet),
            ));
        }
    }
    if view.success {
        panel = panel.with_fixed(hint(&t("transfer.nativeReceiveSuccess")));
    }
    if let Some(error) = &view.message {
        panel = panel.with_fixed(hint(&format!(
            "{}: {}",
            t("transfer.nativeReceiveFailure"),
            error,
        )));
    }
    (
        NewWidget::new(panel).with_props(
            PropertySet::new()
                .with(Gap::new(Length::px(size::SPACE_4)))
                .with(Padding::all(Length::px(size::DIALOG_PADDING))),
        ),
        actions,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revoked_contacts_have_no_receive_action() {
        let contacts = TransportContactView {
            rows: vec![TransportContactRow {
                full_endpoint_id: "c".repeat(64),
                revoked: true,
            }],
            error: None,
        };
        let (_, actions) = render(&NativeReceiveView::default(), &contacts);
        assert!(actions.is_empty());
    }

    #[test]
    fn confirmed_sender_has_separate_internet_receive_action() {
        let contacts = TransportContactView {
            rows: vec![TransportContactRow {
                full_endpoint_id: "d".repeat(64),
                revoked: false,
            }],
            error: None,
        };
        let (_, actions) = render(&NativeReceiveView::default(), &contacts);
        assert_eq!(actions.len(), 2);
        assert!(matches!(&actions[0].1, NativeReceiveAction::Start(_)));
        assert!(matches!(
            &actions[1].1,
            NativeReceiveAction::StartInternet(_)
        ));
    }
}
