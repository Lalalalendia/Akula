//! Explicit one-file Native LAN send with pinned Iroh QUIC.
//! There is no external CLI subprocess and no background service.

use super::super::App;
use crate::ui::transfer::NativeSendView;
use akula_staya_integration::one_file_lan::native_send_one;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

enum Event {
    Listening(Vec<SocketAddr>),
    Finished(Result<(), String>),
}

#[derive(Default)]
pub(in crate::app) struct NativeSender {
    pub(super) view: NativeSendView,
    rx: Option<Receiver<Event>>,
}

impl NativeSender {
    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        loop {
            let event = self.rx.as_ref().map(|receiver| receiver.try_recv());
            match event {
                Some(Ok(Event::Listening(addresses))) => {
                    self.view.listening = Some(
                        addresses
                            .into_iter()
                            .map(|address| {
                                // Wildcard binds are not remote-reachable IPs.
                                if address.ip().is_unspecified() {
                                    format!("UDP {} (choose LAN IP)", address.port())
                                } else {
                                    address.to_string()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" · "),
                    );
                    changed = true;
                }
                Some(Ok(Event::Finished(result))) => {
                    self.view.busy = false;
                    self.view.success = result.is_ok();
                    self.view.message = result.err().map(|error| {
                        error.chars().filter(|ch| !ch.is_control()).take(240).collect()
                    });
                    self.rx = None;
                    changed = true;
                    break;
                }
                Some(Err(TryRecvError::Empty)) | None => break,
                Some(Err(TryRecvError::Disconnected)) => {
                    self.view.busy = false;
                    self.view.message = Some("sender worker disconnected".into());
                    self.rx = None;
                    changed = true;
                    break;
                }
            }
        }
        changed
    }

    pub(super) fn start(
        &mut self,
        peer: String,
        source: PathBuf,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        self.view = NativeSendView {
            busy: true,
            ..NativeSendView::default()
        };
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("create LAN runtime: {error}"));
            let result = runtime.and_then(|runtime| {
                runtime.block_on(native_send_one(peer, source, |bound| {
                    let _ = tx.send(Event::Listening(bound));
                    wake();
                }))
            });
            let _ = tx.send(Event::Finished(result));
            wake();
        });
    }
}

impl App {
    pub(super) fn begin_native_send(&mut self, peer: &str) -> bool {
        if self.transfer.sender.view.busy {
            return false;
        }
        // The UI is not a trust authority. The transport rechecks the
        // protected OS pin and TLS remote endpoint before reading bytes.
        let confirmed = self.transfer.transport_contacts.rows.iter().any(|entry| {
            !entry.revoked && entry.full_endpoint_id == peer
        });
        if !confirmed || self.transfer.drafts.len() != 1 || self.transfer.drafts[0].paused {
            return false;
        }
        self.transfer.sender.start(
            peer.to_owned(),
            self.transfer.drafts[0].path.clone(),
            Arc::clone(&self.workspace.wake),
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_is_idle_before_explicit_user_action() {
        let sender = NativeSender::default();
        assert!(!sender.view.busy);
        assert!(!sender.view.success);
        assert!(sender.view.listening.is_none());
    }
}
