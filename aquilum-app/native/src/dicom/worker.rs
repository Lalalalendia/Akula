//! Bounded, cancellable, read-only DICOM I/O worker.
//! The Masonry event thread never opens or decodes pixel data.
use super::{RenderedFrame, Study};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    Frame(i32),
    Series(i32),
}

pub struct FrameUpdate {
    pub label: String,
    pub frame: Option<RenderedFrame>,
    pub error: Option<String>,
}

pub struct DicomWorker {
    requests: SyncSender<Navigation>,
    updates: Receiver<FrameUpdate>,
    cancelled: Arc<AtomicBool>,
}

impl DicomWorker {
    pub fn start(vault: PathBuf, source: PathBuf) -> Self {
        // At most one request waiting while a frame is being decoded.
        // A busy reader never accumulates an unbounded key-repeat queue.
        let (requests, pending) = mpsc::sync_channel::<Navigation>(1);
        let (updates, results) = mpsc::sync_channel::<FrameUpdate>(2);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        std::thread::spawn(move || {
            let mut study = match Study::open(&vault, &source) {
                Ok(study) => study,
                Err(error) => {
                    if !stop.load(Ordering::Relaxed) {
                        let _ = updates.send(FrameUpdate { label: "DICOM".into(), frame: None, error: Some(error) });
                    }
                    return;
                }
            };
            if stop.load(Ordering::Relaxed) { return; }
            if !publish(&study, &updates) { return; }
            while let Ok(action) = pending.recv() {
                if stop.load(Ordering::Relaxed) { return; }
                let previous = study.clone();
                let moved = match action {
                    Navigation::Frame(delta) => study.advance_frame(delta),
                    Navigation::Series(delta) => study.advance_series(delta),
                };
                if !moved {
                    if updates.send(FrameUpdate { label: study.label(), frame: None, error: None }).is_err() { return; }
                    continue;
                }
                // Keep the previous valid frame and index if media I/O fails.
                match study.render() {
                    Ok(frame) => {
                        if updates.send(FrameUpdate { label: study.label(), frame: Some(frame), error: None }).is_err() { return; }
                    }
                    Err(error) => {
                        study = previous;
                        if updates.send(FrameUpdate { label: study.label(), frame: None, error: Some(error) }).is_err() { return; }
                    }
                }
            }
        });
        Self { requests, updates: results, cancelled }
    }

    pub fn request(&self, action: Navigation) -> bool {
        self.requests.try_send(action).is_ok()
    }

    pub fn poll(&self) -> Result<Option<FrameUpdate>, &'static str> {
        match self.updates.try_recv() {
            Ok(update) => Ok(Some(update)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err("DICOM: фоновое чтение завершено"),
        }
    }
}

fn publish(study: &Study, tx: &SyncSender<FrameUpdate>) -> bool {
    let update = match study.render() {
        Ok(frame) => FrameUpdate { label: study.label(), frame: Some(frame), error: None },
        Err(error) => FrameUpdate { label: study.label(), frame: None, error: Some(error) },
    };
    tx.send(update).is_ok()
}

impl Drop for DicomWorker {
    fn drop(&mut self) {
        // The worker exits after its current bounded read completes. Never
        // join here: closing a tab must not wait for an optical drive.
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_open_reports_missing_file_without_blocking_ui() {
        let directory = tempfile::tempdir().unwrap();
        let worker = DicomWorker::start(
            directory.path().to_path_buf(),
            directory.path().join("missing.dcm")
        );
        for _ in 0..150 {
            match worker.poll() {
                Ok(Some(event)) => {
                    assert!(event.error.is_some());
                    assert!(event.frame.is_none());
                    return;
                }
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                Err(e) => panic!("missing-file result lost: {e}"),
            }
        }
        panic!("background read did not respond within test budget");
    }

    #[test]
    fn disposing_unfinished_worker_does_not_block_the_caller() {
        let dir = tempfile::tempdir().unwrap();
        let worker = DicomWorker::start(dir.path().to_path_buf(), dir.path().join("absent.dcm"));
        drop(worker);
    }
}
