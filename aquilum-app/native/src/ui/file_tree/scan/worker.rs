use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use super::super::model::Counts;

type Wake = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub(in crate::ui::file_tree) struct CountScan {
    receiver: Option<mpsc::Receiver<Counts>>,
    pending: bool,
}

impl CountScan {
    pub fn idle(&self) -> bool {
        self.receiver.is_none()
    }

    pub fn request(&mut self, path: PathBuf, wake: Wake) {
        if self.receiver.is_some() {
            self.pending = true;
            return;
        }
        self.start(path, wake);
    }

    fn start(&mut self, path: PathBuf, wake: Wake) {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            if sender.send(Counts::scan(&path)).is_ok() {
                wake();
            }
        });
        self.receiver = Some(receiver);
    }

    pub fn poll(&mut self, path: &Path, wake: Wake) -> Option<Counts> {
        let counts = self.receiver.as_ref()?.try_recv().ok()?;
        self.receiver = None;
        if self.pending {
            self.pending = false;
            self.start(path.to_path_buf(), wake);
            return None;
        }
        Some(counts)
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, target_os = "windows"))]
mod benchmark;
