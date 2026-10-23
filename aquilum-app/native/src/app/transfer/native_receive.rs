//! Native one-file receive: explicit user action, private Inbox only.
//! Clipboard supplies numeric LAN endpoint, not peer identity or write rights.

use super::super::App;
use crate::ui::transfer::NativeReceiveView;
use akula_staya_integration::one_file_lan::{native_receive_one, validate_direct_lan_address};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

#[derive(Default)]
pub(in crate::app) struct NativeReceiver {
    pub(super) view: NativeReceiveView,
    rx: Option<Receiver<Result<(), String>>>,
    stop: Option<Arc<AtomicBool>>,
}

fn address_from_text(text: &str) -> Result<SocketAddr, String> {
    if text.len() > 128 {
        return Err("LAN address too long".into());
    }
    let address = text
        .trim()
        .parse::<SocketAddr>()
        .map_err(|_| "copy a numeric LAN IP:UDP_PORT, not a hostname")?;
    validate_direct_lan_address(address)?;
    Ok(address)
}

impl NativeReceiver {
    pub(super) fn cancel(&mut self) {
        if let Some(flag) = &self.stop {
            flag.store(true, Ordering::Release);
            self.view.canceling = true;
        }
    }

    pub(super) fn poll(&mut self) -> bool {
        let result = self.rx.as_ref().map(|receiver| receiver.try_recv());
        match result {
            Some(Ok(result)) => {
                self.view.busy = false;
                self.view.canceling = false;
                self.view.success = result.is_ok();
                self.view.message = result.err().map(|message| {
                    message
                        .chars()
                        .filter(|character| !character.is_control())
                        .take(240)
                        .collect()
                });
                self.rx = None;
                self.stop = None;
                true
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.view.busy = false;
                self.view.canceling = false;
                self.view.success = false;
                self.view.message = Some("receiver worker disconnected".into());
                self.rx = None;
                self.stop = None;
                true
            }
            _ => false,
        }
    }

    fn start(&mut self, peer: String, address: SocketAddr, wake: Arc<dyn Fn() + Send + Sync>) {
        let (tx, rx) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        self.rx = Some(rx);
        self.stop = Some(Arc::clone(&stopped));
        self.view = NativeReceiveView {
            busy: true,
            ..NativeReceiveView::default()
        };
        thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("create LAN runtime: {error}"));
            let result = runtime.and_then(|runtime| {
                runtime.block_on(async {
                    tokio::select! {
                        result = native_receive_one(peer, address) => result,
                        _ = async {
                            while !stopped.load(Ordering::Acquire) {
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                        } => Err("cancelled by local user".to_owned()),
                    }
                })
            });
            let _ = tx.send(result);
            wake();
        });
    }
}

impl Drop for NativeReceiver {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl App {
    pub(super) fn begin_native_receive(&mut self, peer: &str) -> bool {
        if self.transfer.receiver.view.busy {
            return false;
        }
        // UI contacts are only a prefilter: transport authenticates/reloads pin.
        let confirmed = self
            .transfer
            .transport_contacts
            .rows
            .iter()
            .any(|contact| !contact.revoked && contact.full_endpoint_id == peer);
        if !confirmed {
            return false;
        }
        let selected = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|_| "could not read numeric LAN address from clipboard".to_string())
            .and_then(|value| address_from_text(&value));
        match selected {
            Ok(address) => {
                self.transfer.receiver.start(
                    peer.to_owned(),
                    address,
                    Arc::clone(&self.workspace.wake),
                );
            }
            Err(error) => {
                self.transfer.receiver.view.success = false;
                self.transfer.receiver.view.message = Some(error);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_address_requires_private_numeric_lan_ip() {
        assert!(address_from_text("192.168.1.40:42982").is_ok());
        assert!(address_from_text(" [::1]:4567 ").is_ok());
        assert!(address_from_text("example.com:42982").is_err());
        assert!(address_from_text("0.0.0.0:42982").is_err());
        assert!(address_from_text("8.8.8.8:42982").is_err());
        assert!(address_from_text("192.168.1.40:0").is_err());
        assert!(address_from_text(&"A".repeat(129)).is_err());
    }

    #[test]
    fn native_receiver_does_not_start_during_app_launch() {
        let receiver = NativeReceiver::default();
        assert!(!receiver.view.busy);
        assert!(!receiver.view.success);
        assert!(receiver.rx.is_none());
        assert!(receiver.stop.is_none());
    }
}
