//! Windows-only, read-only "Verify SSH" action. This is not an installer.
//!
//! The worker rechecks the SAVED independently pinned host public key against
//! actual signed OpenSSH KEX, then reads fixed typed host facts. The UI thread
//! never waits for SSH; a stale reply from an edited draft is discarded.
//! No client SSH secret, remote write, service operation or consent is stored.
#![cfg(all(windows, feature = "server-readonly-preflight"))]

use super::{App, Effect, Popup, Section};
use aquilum_core::server_bootstrap_ssh_windows_lab::WindowsOpenSshPinnedProbe;
use aquilum_core::settings::models::ServerBootstrapDraft;
use crate::ui::settings::schema::ServerProbeView;
use crate::ui::settings::schema::server::identity::probe_target;
use masonry::app::RenderRoot;
use std::sync::Arc;

mod facts;

/// One bounded probe result, scoped to the exact public settings snapshot.
/// This is session-only state, NOT a persistent "installed" receipt.
pub(in crate::app) struct ProbeReport {
    draft: ServerBootstrapDraft,
    view: ServerProbeView,
}

impl App {
    pub(in crate::app) fn request_server_probe(&mut self) -> Effect {
        let draft = self.workspace.core.settings.get_config().server_setup;
        if probe_target(&draft).is_none() {
            self.server_probe.observation = Some((draft, ServerProbeView::Failed));
            return Effect::Content;
        }
        if self.server_probe.observation.as_ref().is_some_and(|(current, status)| {
            current == &draft && matches!(status, ServerProbeView::Checking)
        }) {
            return Effect::None;
        }
        // A second click cannot start a duplicate probe for the same draft.
        self.server_probe.observation = Some((draft.clone(), ServerProbeView::Checking));
        let sender = self.server_probe.sender.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let view = (|| {
                let target = probe_target(&draft)?;
                let facts = if draft.host_public_key.is_empty() {
                    WindowsOpenSshPinnedProbe::inspect_readonly_fingerprint_target(&target).ok()?
                } else {
                    WindowsOpenSshPinnedProbe::inspect_readonly_target(
                        &target, &draft.host_public_key,
                    ).ok()?
                };
                Some(facts::classify(facts))
            })().unwrap_or(ServerProbeView::Failed);
            let _ = sender.send(ProbeReport { draft, view });
            wake();
        });
        Effect::Content
    }

    pub(in crate::app) fn on_server_probe(&mut self, root: &mut RenderRoot) {
        let mut changed = false;
        while let Ok(result) = self.server_probe.receiver.try_recv() {
            // A changed host, user, port, public key or SHA-256 makes the
            // previous observation stale, including if a reply was delayed.
            let current = self.workspace.core.settings.get_config().server_setup;
            if result.draft != current {
                continue;
            }
            let pending = self.server_probe.observation.as_ref().is_some_and(|(draft, state)| {
                draft == &current && matches!(state, ServerProbeView::Checking)
            });
            if !pending {
                continue;
            }
            self.server_probe.observation = Some((result.draft, result.view));
            changed = true;
        }
        if changed && matches!(
            &self.popup,
            Some(Popup::Settings { dialog, .. }) if dialog.section == Section::Server
        ) {
            self.refresh_settings(root);
        }
    }
}
