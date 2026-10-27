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
    #[test]
    fn last_refresh_wins_after_files_change_during_a_burst() {
        use std::time::{Duration, Instant};

        let directory = crate::test_dir::tempdir();
        let root = directory.path();
        std::fs::create_dir(root.join("old_folder")).unwrap();
        std::fs::write(root.join("old_folder/old.md"), "old").unwrap();
        let stale = Counts::scan(root);
        let stale_total = stale.total(root);

        let (sender, receiver) = mpsc::channel();
        sender.send(stale).unwrap();
        let mut coordinator = CountScan { receiver: Some(receiver), pending: false };
        let wake: Wake = Arc::new(|| {});
        for _ in 0..100 {
            coordinator.request(root.to_path_buf(), Arc::clone(&wake));
        }
        std::fs::remove_dir_all(root.join("old_folder")).unwrap();
        std::fs::create_dir(root.join("new_folder")).unwrap();
        std::fs::write(root.join("new_folder/first.md"), "first").unwrap();
        std::fs::write(root.join("new_folder/second.md"), "second").unwrap();

        assert!(coordinator.poll(root, Arc::clone(&wake)).is_none());
        let deadline = Instant::now() + Duration::from_secs(10);
        let refreshed = loop {
            if let Some(counts) = coordinator.poll(root, Arc::clone(&wake)) {
                break counts;
            }
            assert!(Instant::now() < deadline, "final count scan never completed");
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(coordinator.idle());
        assert_ne!(refreshed.total(root), stale_total);
        assert!(!refreshed.knows(&root.join("old_folder")));
        assert!(refreshed.knows(&root.join("new_folder")));
        assert_eq!(refreshed.total(root), Counts::scan(root).total(root));
    }

}

#[cfg(all(test, target_os = "windows"))]
mod benchmark;
