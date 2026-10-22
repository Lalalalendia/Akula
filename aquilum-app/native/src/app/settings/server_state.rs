//! Feature-gated SSH probe state, separate from window's global state.
use super::{App, Effect};
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
