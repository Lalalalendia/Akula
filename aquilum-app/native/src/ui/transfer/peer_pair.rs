//! User-mediated manual public-key pinning in File transfers.
//! A peer pin is only a future QUIC identity expectation, never a file grant.

use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;

use super::{details, hint};
use crate::i18n::t;
use crate::ui::editor::TextEditor;
use crate::ui::settings::controls::InputBox;
use crate::ui::style;
use crate::ui::tokens::size;
use crate::ui::widgets::{TextButton, Variant};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PeerPhase {
    #[default]
    Idle,
    Working,
    NotFound,
    Confirmed,
    Revoked,
    ConfirmRevoke(String),
    Error(String),
}

#[derive(Clone, Debug, Default)]
pub struct PeerView {
    pub observed_id: String,
    pub separately_verified_id: String,
    pub phase: PeerPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerAction {
    Confirm,
    Inspect,
    RequestRevoke,
    Revoke,
    CancelRevoke,
}

pub struct PeerControls {
    pub observed_field: WidgetId,
    pub verified_field: WidgetId,
    pub actions: Vec<(WidgetId, PeerAction)>,
}

fn input(value: &str, disabled: bool) -> NewWidget<TextEditor> {
    NewWidget::new(
        TextEditor::single_line(value).styled(|| style::ui_text(size::INPUT_CONTENT_HEIGHT as f32)),
    )
    .with_class(style::FIELD)
    .disabled(disabled)
}

fn button(
    actions: &mut Vec<(WidgetId, PeerAction)>,
    caption: String,
    action: PeerAction,
) -> NewWidget<TextButton> {
    let widget = NewWidget::new(TextButton::new(caption, Variant::Ghost));
    actions.push((widget.id(), action));
    widget
}

pub(super) fn render(view: &PeerView) -> (NewWidget<Flex>, PeerControls) {
    let busy = view.phase == PeerPhase::Working;
    let observed = input(&view.observed_id, busy);
    let verified = input(&view.separately_verified_id, busy);
    let (observed_field, verified_field) = (observed.id(), verified.id());
    let mut actions = Vec::new();

    let mut column = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.pairTitle")))
        .with_fixed(hint(&t("transfer.pairNoNetwork")))
        .with_fixed(hint(&t("transfer.pairObserved")))
        .with_fixed(InputBox::new(observed, size::INPUT_WIDTH))
        .with_fixed(hint(&t("transfer.pairVerified")))
        .with_fixed(InputBox::new(verified, size::INPUT_WIDTH));
    match &view.phase {
        PeerPhase::Working => {
            column = column.with_fixed(hint(&t("transfer.pairWorking")));
        }
        PeerPhase::ConfirmRevoke(id) => {
            column = column
                .with_fixed(hint(&format!("{} {id}", t("transfer.pairRevokeWarning"))))
                .with_fixed(button(
                    &mut actions,
                    t("transfer.pairConfirmRevoke"),
                    PeerAction::Revoke,
                ))
                .with_fixed(button(
                    &mut actions,
                    t("transfer.pairCancel"),
                    PeerAction::CancelRevoke,
                ));
        }
        state => {
            let state_text = match state {
                PeerPhase::Idle => t("transfer.pairIdle"),
                PeerPhase::NotFound => t("transfer.pairNotFound"),
                PeerPhase::Confirmed => t("transfer.pairConfirmed"),
                PeerPhase::Revoked => t("transfer.pairRevoked"),
                PeerPhase::Error(error) => error.clone(),
                PeerPhase::Working | PeerPhase::ConfirmRevoke(_) => unreachable!(),
            };
            column = column
                .with_fixed(hint(&state_text))
                .with_fixed(button(
                    &mut actions,
                    t("transfer.pairConfirm"),
                    PeerAction::Confirm,
                ))
                .with_fixed(button(
                    &mut actions,
                    t("transfer.pairInspect"),
                    PeerAction::Inspect,
                ));
            if *state == PeerPhase::Confirmed {
                column = column.with_fixed(button(
                    &mut actions,
                    t("transfer.pairRequestRevoke"),
                    PeerAction::RequestRevoke,
                ));
            }
        }
    }
    let widget = NewWidget::new(column).with_props(
        PropertySet::new()
            .with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING))),
    );
    (
        widget,
        PeerControls {
            observed_field,
            verified_field,
            actions,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_pairing_has_distinct_unsynchronized_inputs() {
        let (_, controls) = render(&PeerView::default());
        assert_ne!(controls.observed_field, controls.verified_field);
        assert_eq!(controls.actions.len(), 2);
        assert!(!controls.actions.iter().any(|(_, action)| {
            matches!(action, PeerAction::Revoke | PeerAction::RequestRevoke)
        }));
    }

    #[test]
    fn revocation_requires_separate_confirmation_surface() {
        let (_, requested) = render(&PeerView {
            phase: PeerPhase::Confirmed,
            ..PeerView::default()
        });
        assert!(
            requested
                .actions
                .iter()
                .any(|(_, action)| { *action == PeerAction::RequestRevoke })
        );
        let (_, confirm) = render(&PeerView {
            phase: PeerPhase::ConfirmRevoke("4".repeat(64)),
            ..PeerView::default()
        });
        assert_eq!(confirm.actions.len(), 2);
        assert!(
            confirm
                .actions
                .iter()
                .any(|(_, action)| *action == PeerAction::Revoke)
        );
        assert!(
            confirm.actions.iter().all(|(_, action)| {
                !matches!(action, PeerAction::Confirm | PeerAction::Inspect)
            })
        );
    }

    #[test]
    fn busy_state_cannot_submit_more_commands() {
        let (_, controls) = render(&PeerView {
            phase: PeerPhase::Working,
            ..PeerView::default()
        });
        assert!(controls.actions.is_empty());
    }
}
