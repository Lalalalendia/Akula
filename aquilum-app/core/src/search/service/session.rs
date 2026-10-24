use super::*;
use crate::search::metadata;

impl SearchService {
    pub(crate) fn ensure_running(&self) -> Result<(), SearchError> {
        if self.stopped.load(Ordering::Acquire) {
            Err(SearchError::task("search service already shut down"))
        } else {
            Ok(())
        }
    }

    /// Stop all search indexing before the host starts a new session against
    /// the same Tantivy directory. The native search dialog holds an Arc<Core>
    /// in its request thread and can outlive its App object; dropping the UI
    /// alone does not guarantee that its IndexWriter lock is released.
    ///
    /// Serialize with index preparation, take ownership under short locks,
    /// then synchronously join worker threads with all slots unlocked.
    /// Repeated shutdown is harmless.
    pub fn shutdown(&self) {
        let _guard = self.prepare_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // A late UI search or MCP guest request can still own a cloned Core.
        // Forbid re-creating IndexWriter after the shutdown fence.
        self.stopped.store(true, Ordering::Release);
        let active = self.active.write().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        let guest = self.guest.write().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        if let Some(index) = active {
            index.stop();
        }
        if let Some(guest) = guest {
            guest.index.stop();
        }
        clear_if(&self.analysis_graph, |_| true);
        clear_if(&self.render_graph, |_| true);
    }

    pub fn prepare(&self, workspace: &str) -> Result<SearchIndexStatus, SearchError> {
        let _guard = self.prepare_lock.lock().map_err(SearchError::task)?;
        self.ensure_running()?;
        let root = workspace_root(workspace)?;
        if let Some(status) = self.status_for_root(&root) {
            return Ok(status);
        }
        let opened = match self.take_guest(&root)? {
            Some(guest) => {
                guest.visible.store(true, Ordering::Relaxed);
                guest
            }
            None => self.open_index(root, true)?,
        };
        let progress = Arc::clone(&opened.progress);
        let generation = opened.generation;
        let previous = self.active.write().map_err(SearchError::task)?.replace(opened);
        if let Some(previous) = previous {
            previous.stop();
        }
        clear_if(&self.analysis_graph, |_| true);
        clear_if(&self.render_graph, |_| true);
        let snapshot = progress.snapshot();
        (self.index_notifier)(IndexRevision {
            workspace_path: workspace.to_owned(),
            generation,
            revision: snapshot.revision,
        });
        Ok(snapshot)
    }

    pub fn open_index(&self, root: PathBuf, visible: bool) -> Result<OpenIndex, SearchError> {
        self.ensure_running()?;
        fs::create_dir_all(&self.base_directory)?;
        let key = blake3::hash(root.to_string_lossy().as_bytes()).to_hex();
        let storage = self.base_directory.join(key.as_str());
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let index_dir = storage.join(format!("index-v{}", crate::search::ANALYZER_VERSION));
        let index = Arc::new(SearchIndex::open(&index_dir)?);
        remove_stale_indexes(&storage, &index_dir);
        let progress = Arc::new(IndexProgress::new(index.document_count(), generation));
        let metadata_path = storage.join("documents.sqlite3");
        drop(metadata::open(&metadata_path)?);
        let visible = Arc::new(AtomicBool::new(visible));
        let worker = WorkerHandle::spawn(
            root.clone(),
            Arc::clone(&index),
            metadata_path.clone(),
            Arc::clone(&progress),
            generation,
            self.refresh_notifier(root.clone(), generation, Arc::clone(&visible)),
        )?;
        Ok(OpenIndex {
            root,
            index,
            worker,
            progress,
            metadata_path,
            generation,
            visible,
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn shutdown_releases_index_writer_for_next_core_while_old_clone_is_alive() {
        use crate::search::models::SearchIndexState;
        use crate::search::query::Request;
        use std::path::PathBuf;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let temporary = tempfile::tempdir().unwrap();
        let vault = temporary.path().join("notes");
        fs::create_dir_all(&vault).unwrap();
        fs::write(vault.join("test.md"), "# One\n\nshutdownsearchmarker\n").unwrap();
        let workspace = vault.to_string_lossy().into_owned();
        let store = temporary.path().join("persistent-index");
        let make_search = || {
            SearchService::new(
                store.clone(),
                Arc::new(|_: Vec<PathBuf>| {}),
                Arc::new(|_| {}),
            )
        };
        let old = make_search();
        old.prepare(&workspace).expect("index should open");
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let status = old.status(Some(&workspace));
            if status.state == SearchIndexState::Ready
                && !status.updating && status.indexed_documents >= 1
            {
                break;
            }
            assert!(Instant::now() < deadline, "first index never completed");
            std::thread::sleep(Duration::from_millis(10));
        }

        // Simulates a search dialog background task holding SearchService
        // after the native App and main Core shutdown begin.
        let stale_client = old.clone();
        old.shutdown();
        assert!(stale_client.active_root().is_none(), "shutdown left an active index");
        stale_client.shutdown(); // idempotent across cloned clients
        assert!(stale_client.prepare(&workspace).is_err(),
            "late search task reopened an IndexWriter after shutdown");
        assert!(stale_client.open_guest(&workspace).is_err(),
            "late guest request reopened an IndexWriter after shutdown");

        // This is a new app session with the same real filesystem/SQLite/Tantivy
        // paths. LockBusy must not be hidden or delayed with an arbitrary sleep.
        let restarted = make_search();
        restarted.prepare(&workspace).expect("new app cannot reopen the old index");
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let status = restarted.status(Some(&workspace));
            if status.state == SearchIndexState::Ready && !status.updating
                && status.indexed_documents >= 1
            {
                break;
            }
            assert!(Instant::now() < deadline, "restarted index never completed");
            std::thread::sleep(Duration::from_millis(10));
        }
        let result = restarted
            .search(&workspace, &Request::new("shutdownsearchmarker"))
            .expect("query should not see a LockBusy or stale session");
        assert_eq!(result.results.len(), 1, "saved source is not searchable");
        restarted.shutdown();
    }

}
