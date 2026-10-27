use std::path::Path;
use std::sync::Arc;

use super::super::FileTree;
use super::super::model::Counts;

fn should_start(running: bool, pending: &mut bool) -> bool {
    if running {
        *pending = true;
        false
    } else {
        true
    }
}

impl FileTree {
    pub(in crate::ui::file_tree) fn count_notes(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        let Some(root) = self.model.root().map(Path::to_path_buf) else { return };
        if !should_start(self.counting.is_some(), &mut self.pending_count_scan) {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if sender.send(Counts::scan(&root)).is_ok() {
                wake();
            }
        });
        self.counting = Some(receiver);
    }
}

#[cfg(test)]
mod tests {
    use super::should_start;

    #[test]
    fn repeated_requests_while_running_coalesce_to_one_restart() {
        let mut pending = false;
        assert!(should_start(false, &mut pending));
        for _ in 0..100 {
            assert!(!should_start(true, &mut pending));
        }
        assert!(pending);
        pending = false; // Poll discards the stale result and starts one new pass.
        assert!(should_start(false, &mut pending));
        assert!(!pending);
    }

    #[test]
    fn idle_request_starts_without_marking_a_rescan() {
        let mut pending = false;
        assert!(should_start(false, &mut pending));
        assert!(!pending);
    }
}
