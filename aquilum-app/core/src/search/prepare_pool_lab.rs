// Research-only, bounded alternative to per-batch scoped threads.
// This module is enabled by the opt-in index-pool-lab Cargo feature.
// No production caller chooses it without the test-only environment gate.
use super::{prepare_one, PendingPrepare, SearchError};
use super::index_document::{PreparedFile, Probe};
use std::sync::mpsc;
use std::thread;

type Prepared = (String, Probe, Option<PreparedFile>);

struct Worker {
    sender: Option<mpsc::SyncSender<Vec<PendingPrepare>>>,
    receiver: mpsc::Receiver<Vec<Prepared>>,
    join: Option<thread::JoinHandle<()>>,
}

pub(super) struct PreparePool {
    workers: Vec<Worker>,
}

impl PreparePool {
    pub(super) fn spawn() -> Result<Self, SearchError> {
        let count = thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(4)
            .clamp(1, 8);
        let mut workers = Vec::with_capacity(count);
        for id in 0..count {
            let (sender, incoming) = mpsc::sync_channel::<Vec<PendingPrepare>>(1);
            let (outgoing, receiver) = mpsc::sync_channel::<Vec<Prepared>>(1);
            let join = thread::Builder::new()
                .name(format!("akula-index-prep-lab-{id}"))
                .spawn(move || {
                    while let Ok(chunk) = incoming.recv() {
                        let prepared = chunk.into_iter().map(prepare_one).collect();
                        if outgoing.send(prepared).is_err() {
                            break;
                        }
                    }
                })
                .map_err(SearchError::task)?;
            workers.push(Worker {
                sender: Some(sender),
                receiver,
                join: Some(join),
            });
        }
        Ok(Self { workers })
    }

    pub(super) fn prepare(
        &mut self,
        mut batch: Vec<PendingPrepare>,
    ) -> Result<Vec<Prepared>, SearchError> {
        if batch.len() < 2 || self.workers.len() == 1 {
            return Ok(batch.into_iter().map(prepare_one).collect());
        }
        let expected = batch.len();
        let chunk_size = expected.div_ceil(self.workers.len());
        let mut dispatched = 0;
        for worker in &self.workers {
            if batch.is_empty() {
                break;
            }
            let take = batch.len().min(chunk_size);
            let chunk = batch.drain(..take).collect::<Vec<_>>();
            worker
                .sender
                .as_ref()
                .ok_or_else(|| SearchError::task("index preparation sender closed"))?
                .send(chunk)
                .map_err(SearchError::task)?;
            dispatched += 1;
        }

        // Each receiver is read in chunk order. No documents may be silently
        // dropped if a worker panics or disconnects; stop indexing with Error.
        let mut result = Vec::with_capacity(expected);
        for worker in self.workers.iter().take(dispatched) {
            result.extend(worker.receiver.recv().map_err(SearchError::task)?);
        }
        if result.len() != expected {
            return Err(SearchError::task("index preparation lost a document"));
        }
        Ok(result)
    }
}

impl Drop for PreparePool {
    fn drop(&mut self) {
        // Close all input queues before joining; no worker remains after
        // full_scan(), including when the scan is cancelled or fails.
        for worker in &mut self.workers {
            worker.sender.take();
        }
        for worker in &mut self.workers {
            if let Some(join) = worker.join.take() {
                let _ = join.join();
            }
        }
    }
}
