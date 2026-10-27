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
mod tests {
    use super::*;

    #[test]
    fn repeated_requests_keep_only_one_pending_scan() {
        let mut coordinator = CountScan::default();
        let (sender, receiver) = mpsc::channel();
        coordinator.receiver = Some(receiver);
        let wake: Wake = Arc::new(|| {});
        let path = PathBuf::from("synthetic");
        for _ in 0..100 {
            coordinator.request(path.clone(), Arc::clone(&wake));
        }
        assert!(coordinator.pending);
        assert!(sender.send(Counts::default()).is_ok());
        assert!(coordinator.poll(&path, Arc::clone(&wake)).is_none());
        assert!(!coordinator.pending);
        assert!(!coordinator.idle());
    }

    #[test]
    fn finished_scan_yields_result_if_nothing_supersedes_it() {
        let mut coordinator = CountScan::default();
        let (sender, receiver) = mpsc::channel();
        coordinator.receiver = Some(receiver);
        assert!(sender.send(Counts::default()).is_ok());
        assert!(coordinator.poll(Path::new("synthetic"), Arc::new(|| {})).is_some());
        assert!(coordinator.idle());
    }
}
