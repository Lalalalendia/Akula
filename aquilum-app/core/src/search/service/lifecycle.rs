use super::*;

// Closing the visible vault must also retire its index worker. A stopped file
// watcher alone does not stop the search worker from holding old vault data.
impl SearchService {
    pub fn deactivate(&self) -> Result<(), SearchError> {
        let _guard = self.prepare_lock.lock().map_err(SearchError::task)?;
        let previous = self.active.write().map_err(SearchError::task)?.take();
        if let Some(open) = previous {
            open.visible.store(false, Ordering::Relaxed);
            open.stop();
        }
        clear_if(&self.analysis_graph, |_| true);
        clear_if(&self.render_graph, |_| true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_vault_stops_its_index_and_reopening_starts_new_generation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        fs::create_dir(&root).unwrap();
        let search = SearchService::new(
            dir.path().join("indexes"),
            Arc::new(|_: Vec<PathBuf>| {}),
            Arc::new(|_: IndexRevision| {}),
        );
        let path = root.to_string_lossy();
        search.prepare(&path).unwrap();
        assert_eq!(search.active_root(), Some(fs::canonicalize(&root).unwrap()));
        search.deactivate().unwrap();
        assert!(search.active_root().is_none());
        search.deactivate().unwrap(); // idempotent
        search.prepare(&path).unwrap();
        assert_eq!(search.active_root(), Some(fs::canonicalize(&root).unwrap()));
        search.deactivate().unwrap();
        assert!(search.active_root().is_none());
    }
}
