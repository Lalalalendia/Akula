//! Windows-only, read-only "Verify SSH" action. This is not an installer.
//!
//! The worker rechecks the SAVED independently pinned host public key against
//! actual signed OpenSSH KEX, then reads fixed typed host facts. The UI thread
//! never waits for SSH; a stale reply from an edited draft is discarded.
//! No client SSH secret, remote write, service operation or consent is stored.
#![cfg(all(windows, feature = "server-readonly-preflight"))]

use super::{App, Effect, Popup, Section};
use aquilum_core::server_bootstrap_lab::{MachineArch, SupportedOs};
use aquilum_core::server_bootstrap_ssh_windows_lab::WindowsOpenSshPinnedProbe;
use aquilum_core::settings::models::ServerBootstrapDraft;
use crate::ui::settings::schema::{ServerProbeView, probe_target};
use masonry::app::RenderRoot;
use std::sync::Arc;

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
                let os = match &facts.os {
                    SupportedOs::Ubuntu => "Ubuntu",
                    SupportedOs::Debian => "Debian",
                    SupportedOs::Other => "Other",
                };
                let arch = match &facts.arch {
                    MachineArch::X86_64 => "x86_64",
                    MachineArch::Aarch64 => "aarch64",
                    MachineArch::Other => "other",
                };
                let summary = format!(
                    "{os} / {arch}; {} CPU; {} MiB RAM; {} MiB free disk; systemd {}; sudo {}",
                    facts.cpu_count, facts.ram_mib, facts.free_disk_mib,
                    if facts.has_systemd { "yes" } else { "no" },
                    if facts.has_sudo_rights { "yes" } else { "no" },
                );
                // The peer's read-only facts are NOT an installation grant.
                // A valid signed SSH host KEX can still describe an
                // unsupported Linux runtime; report that distinction.
                let supported = !matches!(facts.os, SupportedOs::Other)
                    && !matches!(facts.arch, MachineArch::Other)
                    && facts.has_systemd && facts.has_sudo_rights
                    && facts.cpu_count > 0 && facts.ram_mib >= 512
                    && facts.free_disk_mib >= 1024;
                Some(if supported {
                    ServerProbeView::Observed { facts: summary }
                } else {
                    ServerProbeView::Unsupported { facts: summary }
                })
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
