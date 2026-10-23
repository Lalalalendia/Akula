//! Feature-gated SSH probe state, separate from window's global state.
use super::{App, Effect, Section};
use aquilum_core::settings::models::ServerBootstrapDraft;
use crate::ui::settings::schema::ServerProbeView;
use masonry::app::RenderRoot;
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
    pub(super) fn server_probe_view(&self, section: Section) -> Option<ServerProbeView> {
        if section != Section::Server {
            return None;
        }
        let current = self.workspace.core.settings.get_config().server_setup;
        self.server_probe.observation.as_ref()
            .filter(|(draft, _)| draft == &current)
            .map(|(_, status)| status.clone())
    }

    pub(in crate::app) fn on_update_and_server_probe(&mut self, root: &mut RenderRoot) {
        self.on_update(root);
        #[cfg(all(windows, feature = "server-readonly-preflight"))]
        self.on_server_probe(root);
    }
    #[cfg(not(all(windows, feature = "server-readonly-preflight")))]
    pub(in crate::app) fn request_server_probe(&mut self) -> Effect {
        Effect::None
    }
}
