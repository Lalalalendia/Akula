//! Explicit local file picker only. No network send.
use super::*;

impl App {
    pub(super) fn choose_transfer_files(&mut self) {
        if self.transfer.choosing {
            return;
        }
        self.transfer.choosing = true;
        let tx = self.transfer_tx.clone();
        let wake = Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let picked = rfd::FileDialog::new().pick_files().unwrap_or_default();
            if tx.send(picked).is_ok() {
                wake();
            }
        });
    }

}
