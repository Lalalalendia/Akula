//! One-file display-only QA consent panel. No file path, opener or grant.
use super::super::*;

pub(super) fn render(
    offer: &OfferConsentView,
) -> (NewWidget<Flex>, Vec<(WidgetId, OfferConsentAction)>) {
    let decline = NewWidget::new(TextButton::new(
        t("transfer.offerConsentDecline"),
        Variant::Ghost,
    ));
    let accept = NewWidget::new(TextButton::new(
        t("transfer.offerConsentAccept"),
        Variant::Primary,
    ));
    let actions = vec![
        (decline.id(), OfferConsentAction::Decline(offer.revision)),
        (accept.id(), OfferConsentAction::Accept(offer.revision)),
    ];
    let panel = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.offerConsentTitle")))
        .with_fixed(hint(&t("transfer.offerConsentWarning")))
        .with_fixed(details(&format!(
            "{}: {}",
            t("transfer.offerConsentSender"),
            offer.sender_endpoint
        )))
        .with_fixed(details(&format!(
            "{}: {}",
            t("transfer.offerConsentOffer"),
            offer.offer_id
        )))
        .with_fixed(details(&format!(
            "{}: {}",
            t("transfer.offerConsentFile"),
            offer.file_label
        )))
        .with_fixed(details(&display_size(offer.bytes)))
        .with_fixed(hint(&format!(
            "{}: {}",
            t("transfer.offerConsentDigest"),
            offer.digest
        )))
        .with_fixed(
            Flex::row()
                .main_axis_alignment(MainAxisAlignment::End)
                .with_fixed(decline)
                .with_fixed(accept),
        );
    (
        NewWidget::new(panel).with_props(
            PropertySet::new()
                .with(Gap::new(Length::px(size::SPACE_4)))
                .with(Padding::all(Length::px(size::DIALOG_PADDING))),
        ),
        actions,
    )
}
