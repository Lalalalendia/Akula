//! First-party local Iroh identity display. It is not peer enrollment,
//! file authority or evidence of a completed network transfer.

use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::{details, hint};
use crate::i18n::t;
use crate::ui::tokens::size;
use crate::ui::widgets::{TextButton, Variant};

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum IdentityPhase {
    #[default]
    Unchecked,
    Working,
    Missing,
    Ready(String),
    Error(String),
}

#[derive(Clone, Debug, Default)]
pub struct IdentityView {
    pub phase: IdentityPhase,
    pub notice: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityAction {
    Load,
    Initialize,
    CopyPublic,
}

fn action_button(
    actions: &mut Vec<(WidgetId, IdentityAction)>,
    label: String,
    action: IdentityAction,
) -> NewWidget<TextButton> {
    let button = NewWidget::new(TextButton::new(label, Variant::Ghost));
    actions.push((button.id(), action));
    button
}

pub(super) fn render(view: &IdentityView) -> (NewWidget<Flex>, Vec<(WidgetId, IdentityAction)>) {
    let mut actions = Vec::new();
    let mut card = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.localIdentity")))
        .with_fixed(hint(&t("transfer.localIdentityNoGrant")));
    match &view.phase {
        IdentityPhase::Unchecked => {
            card = card
                .with_fixed(hint(&t("transfer.localIdentityUnchecked")))
                .with_fixed(action_button(
                    &mut actions,
                    t("transfer.localIdentityLoad"),
                    IdentityAction::Load,
                ));
        }
        IdentityPhase::Working => {
            card = card.with_fixed(hint(&t("transfer.localIdentityWorking")));
        }
        IdentityPhase::Missing => {
            card = card
                .with_fixed(hint(&t("transfer.localIdentityMissing")))
                .with_fixed(action_button(
                    &mut actions,
                    t("transfer.localIdentityCreate"),
                    IdentityAction::Initialize,
                ))
                .with_fixed(action_button(
                    &mut actions,
                    t("transfer.localIdentityLoad"),
                    IdentityAction::Load,
                ));
        }
        IdentityPhase::Ready(endpoint) => {
            card = card
                .with_fixed(details(endpoint))
                .with_fixed(action_button(
                    &mut actions,
                    t("transfer.localIdentityCopy"),
                    IdentityAction::CopyPublic,
                ))
                .with_fixed(action_button(
                    &mut actions,
                    t("transfer.localIdentityLoad"),
                    IdentityAction::Load,
                ));
        }
        IdentityPhase::Error(error) => {
            card = card.with_fixed(hint(error)).with_fixed(action_button(
                &mut actions,
                t("transfer.localIdentityLoad"),
                IdentityAction::Load,
            ));
        }
    }
    if let Some(notice) = &view.notice {
        card = card.with_fixed(hint(notice));
    }
    let card = NewWidget::new(card).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    );
    (card, actions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_key_offers_explicit_creation_but_never_network_send() {
        let (_, actions) = render(&IdentityView {
            phase: IdentityPhase::Missing,
            notice: None,
        });
        assert_eq!(actions.len(), 2);
        assert!(
            actions
                .iter()
                .any(|(_, a)| *a == IdentityAction::Initialize)
        );
        assert!(
            actions
                .iter()
                .all(|(_, a)| *a != IdentityAction::CopyPublic)
        );
    }

    #[test]
    fn ready_key_can_only_be_read_or_copied() {
        let (_, actions) = render(&IdentityView {
            phase: IdentityPhase::Ready("a".repeat(64)),
            notice: None,
        });
        assert_eq!(actions.len(), 2);
        assert!(
            actions
                .iter()
                .all(|(_, a)| { matches!(a, IdentityAction::Load | IdentityAction::CopyPublic) })
        );
    }
}
