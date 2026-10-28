//! OS-backed, user-mediated peer key pins. Every stored record is only an
//! expected QUIC public key. The real remote_id MUST be checked during connect.
//! Never interpret a pin as an OfferSession, GET, or File Gate authorization.

use std::sync::{
    Arc,
    mpsc::{Receiver, Sender, channel},
};

use aquilum_core::p2p_device_identity::peer_pin::{self, PeerState};
use masonry::core::{ErasedAction, WidgetId};

use crate::i18n::t;
use crate::ui::editor::TextAction;
use crate::ui::transfer::{PeerAction, PeerControls, PeerPhase, PeerView};
use crate::ui::widgets::Pressed;

enum ResultState {
    State(PeerState),
    Error(String),
}

pub(super) struct PeerUi {
    pub(super) view: PeerView,
    generation: u64,
    tx: Sender<(u64, String, ResultState)>,
    rx: Receiver<(u64, String, ResultState)>,
}

impl Default for PeerUi {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            view: PeerView::default(),
            generation: 0,
            tx,
            rx,
        }
    }
}

fn valid_key(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && id.bytes().any(|b| b != b'0')
}

impl PeerUi {
    fn update_input(&mut self, observed: bool, value: &str) {
        if self.view.phase == PeerPhase::Working {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        // No arbitrary-length GUI text is copied into background jobs.
        let bounded = value.chars().take(128).collect();
        if observed {
            self.view.observed_id = bounded;
        } else {
            self.view.separately_verified_id = bounded;
        }
        self.view.phase = PeerPhase::Idle;
    }

    pub(super) fn handle(
        &mut self,
        controls: &PeerControls,
        id: WidgetId,
        event: &ErasedAction,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> bool {
        if id == controls.observed_field || id == controls.verified_field {
            if let Some(TextAction::Changed(text) | TextAction::Entered(text)) =
                event.downcast_ref::<TextAction>()
            {
                self.update_input(id == controls.observed_field, text);
            }
            // Input editing never auto-confirms a peer or reconstructs the UI.
            return false;
        }
        if event.downcast_ref::<Pressed>().is_none() {
            return false;
        }
        let Some((_, action)) = controls.actions.iter().find(|(button, _)| *button == id) else {
            return false;
        };
        self.act(*action, wake)
    }

    fn act(&mut self, action: PeerAction, wake: Arc<dyn Fn() + Send + Sync>) -> bool {
        if self.view.phase == PeerPhase::Working {
            return false;
        }
        let observed = self.view.observed_id.clone();
        let verified = self.view.separately_verified_id.clone();
        match action {
            PeerAction::CancelRevoke => {
                if matches!(self.view.phase, PeerPhase::ConfirmRevoke(_)) {
                    self.view.phase = PeerPhase::Idle;
                    return true;
                }
                return false;
            }
            PeerAction::RequestRevoke => {
                if self.view.phase != PeerPhase::Confirmed || !valid_key(&observed) {
                    return false;
                }
                self.view.phase = PeerPhase::ConfirmRevoke(observed);
                return true;
            }
            PeerAction::Revoke => {
                let PeerPhase::ConfirmRevoke(ref pending) = self.view.phase else {
                    return false;
                };
                if pending != &observed || observed != verified {
                    self.view.phase = PeerPhase::Error(t("transfer.pairRevokeChanged"));
                    return true;
                }
            }
            PeerAction::Confirm => {
                if !valid_key(&observed) || !valid_key(&verified) || observed != verified {
                    self.view.phase = PeerPhase::Error(t("transfer.pairInvalidComparison"));
                    return true;
                }
            }
            PeerAction::Inspect => {
                if !valid_key(&observed) {
                    self.view.phase = PeerPhase::Error(t("transfer.pairInvalidObserved"));
                    return true;
                }
            }
        }

        // The first value may originate in a network invitation, but the
        // second is typed after checking the real other device or a separate
        // authenticated human channel. This creates an expected key only:
        // the eventual QUIC remote_id must still cryptographically match.
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.view.phase = PeerPhase::Working;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = match action {
                PeerAction::Confirm => peer_pin::confirm_peer(&observed, &verified)
                    .and_then(|_| peer_pin::peer_state(&observed)),
                PeerAction::Inspect => peer_pin::peer_state(&observed),
                PeerAction::Revoke => {
                    peer_pin::revoke_peer(&observed).and_then(|_| peer_pin::peer_state(&observed))
                }
                PeerAction::RequestRevoke | PeerAction::CancelRevoke => unreachable!(),
            }
            .map(ResultState::State)
            .unwrap_or_else(|error| ResultState::Error(error.to_string()));
            if tx.send((generation, observed, result)).is_ok() {
                wake();
            }
        });
        true
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut updated = false;
        while let Ok((generation, observed, result)) = self.rx.try_recv() {
            if generation != self.generation || observed != self.view.observed_id {
                continue;
            }
            self.view.phase = match result {
                ResultState::State(PeerState::NotFound) => PeerPhase::NotFound,
                ResultState::State(PeerState::Confirmed) => PeerPhase::Confirmed,
                ResultState::State(PeerState::Revoked) => PeerPhase::Revoked,
                ResultState::Error(error) => PeerPhase::Error(error),
            };
            updated = true;
        }
        updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_lowercase_full_id_can_be_submitted() {
        assert!(valid_key(&"a".repeat(64)));
        assert!(!valid_key(&"A".repeat(64)));
        assert!(!valid_key(&"0".repeat(64)));
        assert!(!valid_key(&"a".repeat(63)));
        assert!(!valid_key(&"g".repeat(64)));
    }

    #[test]
    fn mismatched_independent_confirmation_never_reaches_os() {
        let mut ui = PeerUi::default();
        ui.view.observed_id = "1".repeat(64);
        ui.view.separately_verified_id = "2".repeat(64);
        assert!(ui.act(PeerAction::Confirm, Arc::new(|| {})));
        assert!(matches!(ui.view.phase, PeerPhase::Error(_)));
        assert_eq!(ui.generation, 0);
    }

    #[test]
    fn no_revocation_without_two_step_review_and_unchanged_id() {
        let mut ui = PeerUi::default();
        ui.view.observed_id = "1".repeat(64);
        ui.view.separately_verified_id = "1".repeat(64);
        assert!(!ui.act(PeerAction::Revoke, Arc::new(|| {})));
        ui.view.phase = PeerPhase::Confirmed;
        assert!(ui.act(PeerAction::RequestRevoke, Arc::new(|| {})));
        assert!(matches!(ui.view.phase, PeerPhase::ConfirmRevoke(_)));
        ui.update_input(true, &"2".repeat(64));
        assert!(matches!(ui.view.phase, PeerPhase::Idle));
        assert!(!ui.act(PeerAction::Revoke, Arc::new(|| {})));
    }

    #[test]
    fn async_status_is_bound_to_matching_generation_and_id() {
        let mut ui = PeerUi::default();
        ui.view.observed_id = "a".repeat(64);
        ui.generation = 3;
        ui.tx
            .send((2, "a".repeat(64), ResultState::State(PeerState::Confirmed)))
            .unwrap();
        ui.tx
            .send((3, "b".repeat(64), ResultState::State(PeerState::Confirmed)))
            .unwrap();
        ui.tx
            .send((3, "a".repeat(64), ResultState::State(PeerState::NotFound)))
            .unwrap();
        assert!(ui.poll());
        assert_eq!(ui.view.phase, PeerPhase::NotFound);
    }
}
