//! Nonblocking UI adapter for the existing Windows Credential Manager-backed
//! Iroh identity. Explicit creation only; never reads or exports a private key.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::i18n::t;
use crate::ui::transfer::{IdentityAction, IdentityPhase, IdentityView};
use aquilum_core::p2p_device_identity::{self, IdentityError};

enum Loaded {
    Missing,
    Ready(String),
    Failure(String),
}

pub(super) struct LocalIdentityUi {
    pub(super) view: IdentityView,
    generation: u64,
    tx: Sender<(u64, Loaded)>,
    rx: Receiver<(u64, Loaded)>,
}

impl Default for LocalIdentityUi {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            view: IdentityView::default(),
            generation: 0,
            tx,
            rx,
        }
    }
}

impl LocalIdentityUi {
    pub(super) fn action(&mut self, action: IdentityAction, wake: Arc<dyn Fn() + Send + Sync>) {
        if matches!(self.view.phase, IdentityPhase::Working) {
            return;
        }
        if action == IdentityAction::CopyPublic {
            self.copy_public();
            return;
        }
        // First-run enrollment is available only AFTER an explicit read found
        // that the credential is absent. The OS store refuses replacements.
        if action == IdentityAction::Initialize
            && !matches!(self.view.phase, IdentityPhase::Missing)
        {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.view.phase = IdentityPhase::Working;
        self.view.notice = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = match action {
                IdentityAction::Load => read_identity(),
                IdentityAction::Initialize => create_identity(),
                IdentityAction::CopyPublic => unreachable!(),
            };
            if tx.send((generation, result)).is_ok() {
                wake();
            }
        });
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((generation, result)) = self.rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            self.view.phase = match result {
                Loaded::Missing => IdentityPhase::Missing,
                Loaded::Ready(public) => IdentityPhase::Ready(public),
                Loaded::Failure(error) => IdentityPhase::Error(error),
            };
            changed = true;
        }
        changed
    }

    fn copy_public(&mut self) {
        let IdentityPhase::Ready(public) = &self.view.phase else {
            return;
        };
        if public.len() != 64 || !public.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.view.notice = Some(t("transfer.localIdentityCopyFail"));
            return;
        }
        self.view.notice = Some(
            match arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.set_text(public.clone()))
            {
                Ok(()) => t("transfer.localIdentityCopied"),
                Err(_) => t("transfer.localIdentityCopyFail"),
            },
        );
    }
}

fn read_identity() -> Loaded {
    match p2p_device_identity::load() {
        Ok(identity) => Loaded::Ready(identity.endpoint_id_hex()),
        Err(IdentityError::NotInitialized) => Loaded::Missing,
        Err(error) => Loaded::Failure(error.to_string()),
    }
}

fn create_identity() -> Loaded {
    match p2p_device_identity::initialize() {
        Ok(identity) => Loaded::Ready(identity.endpoint_id_hex()),
        // An independently running Akula may enroll between read and create.
        // Never replace it or imply that a second identity was generated.
        Err(IdentityError::AlreadyInitialized) => read_identity(),
        Err(error) => Loaded::Failure(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_is_refused_without_prior_missing_state() {
        let mut ui = LocalIdentityUi::default();
        ui.action(IdentityAction::Initialize, Arc::new(|| {}));
        assert!(matches!(ui.view.phase, IdentityPhase::Unchecked));
    }

    #[test]
    fn stale_async_results_cannot_replace_fresh_public_identity() {
        let mut ui = LocalIdentityUi::default();
        ui.generation = 2;
        ui.tx.send((1, Loaded::Ready("b".repeat(64)))).unwrap();
        ui.tx.send((2, Loaded::Ready("a".repeat(64)))).unwrap();
        assert!(ui.poll());
        assert!(matches!(
            ui.view.phase, IdentityPhase::Ready(ref id) if id == &"a".repeat(64)
        ));
    }

    #[test]
    fn a_missing_identity_has_no_copyable_public_material() {
        let mut ui = LocalIdentityUi::default();
        ui.view.phase = IdentityPhase::Missing;
        ui.action(IdentityAction::CopyPublic, Arc::new(|| {}));
        assert!(matches!(ui.view.phase, IdentityPhase::Missing));
        assert!(ui.view.notice.is_none());
    }
}
