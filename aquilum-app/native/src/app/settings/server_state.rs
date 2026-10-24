//! Feature-gated SSH probe state, separate from window's global state.
use super::{App, Effect, Section, Action, Context, SettingsDialog};
use masonry::app::RenderRoot;
use aquilum_core::settings::models::ServerBootstrapDraft;
use crate::ui::settings::schema::ServerProbeView;
#[cfg(all(windows, feature = "server-readonly-preflight"))]
use super::server::ProbeReport;
#[cfg(all(windows, feature = "server-readonly-preflight"))]
use std::sync::mpsc::{channel, Receiver, Sender};

pub(in crate::app) struct ServerProbeState {
    pub(in crate::app) observation: Option<(ServerBootstrapDraft, ServerProbeView)>,
    #[cfg(all(windows, feature = "server-readonly-preflight"))]
    pub(in crate::app) sender: Sender<ProbeReport>,
    #[cfg(all(windows, feature = "server-readonly-preflight"))]
    pub(in crate::app) receiver: Receiver<ProbeReport>,
}
impl Default for ServerProbeState {
    fn default() -> Self {
        #[cfg(all(windows, feature = "server-readonly-preflight"))]
        let (sender, receiver) = channel();
        Self {
            observation: None,
            #[cfg(all(windows, feature = "server-readonly-preflight"))]
            sender,
            #[cfg(all(windows, feature = "server-readonly-preflight"))]
            receiver,
        }
    }
}
impl App {
    /// Refresh updates on every platform; poll signed SSH only on Windows.
    pub(in crate::app) fn poll_update_and_server_probe(&mut self, root: &mut RenderRoot) {
        self.on_update(root);
        #[cfg(all(windows, feature = "server-readonly-preflight"))]
        self.on_server_probe(root);
    }

    pub(super) fn server_probe_view(&self, section: Section) -> Option<ServerProbeView> {
        if section != Section::Server {
            return None;
        }
        let current = self.workspace.core.settings.get_config().server_setup;
        self.server_probe.observation.as_ref()
            .filter(|(draft, _)| draft == &current)
            .map(|(_, status)| status.clone())
    }

    pub(super) fn handle_server_button(
        &mut self, dialog: &mut SettingsDialog, action: Action,
    ) -> Effect {
        match action {
            Action::VerifyServer if dialog.section == Section::Server =>
                self.request_server_probe(),
            Action::ToggleServerAdvanced if dialog.section == Section::Server => {
                dialog.server_advanced = !dialog.server_advanced;
                Effect::Content
            }
            // Nothing can grant installation from the ordinary settings UI.
            _ => Effect::None,
        }
    }

    pub(super) fn dialog_context(&self, dialog: &SettingsDialog) -> Context {
        let mut ctx = self.settings_context(dialog.section, dialog.copied, dialog.trash_page);
        ctx.sync_selected = dialog.sync_selected.clone();
        ctx.sync_notice = dialog.sync_notice.clone();
        ctx.yandex_notice = dialog.yandex_notice.clone();
        ctx.server_advanced = dialog.server_advanced;
        ctx
    }

    #[cfg(not(all(windows, feature = "server-readonly-preflight")))]
    pub(in crate::app) fn request_server_probe(&mut self) -> Effect {
        Effect::None
    }
}
