//! Bounded, token-free offline Notion status for the experimental native tile.
//! Never opens a Notion HTTP client or touches user Vault / File Gate.
#[cfg(not(debug_assertions))]
compile_error!("notion-offline-preview is QA-only and may not ship in a release build");

use std::io::ErrorKind;
use std::path::Path;
use aquilum_core::notion_api_probe::snapshot::SnapshotStore;

#[derive(Clone, Debug)]
pub struct CachedRoot {
    pub id: String,
    pub objects: usize,
    pub links: usize,
    pub titles: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NotionOfflineView {
    pub roots: Vec<CachedRoot>,
    pub error: Option<&'static str>,
}

impl NotionOfflineView {
    pub fn read(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("ui-state.sqlite3");
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == ErrorKind::NotFound => return Self::default(),
            Err(_) => return Self { error: Some("local cache cannot be inspected"), ..Self::default() },
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() =>
                return Self { error: Some("local cache is not a regular file"), ..Self::default() },
            Ok(_) => {}
        }
        let inspect = || -> Result<Vec<CachedRoot>, String> {
            let store = SnapshotStore::open_read_only(&path)?;
            let sources = store.list_sources()?;
            let mut result = Vec::with_capacity(sources.len());
            for info in sources.into_iter().take(16) {
                let titles = store.preview_page_titles(&info.root, 4)?;
                result.push(CachedRoot {
                    id: info.root,
                    objects: info.object_count,
                    links: info.link_count,
                    titles,
                });
            }
            Ok(result)
        };
        match inspect() {
            Ok(roots) => Self { roots, error: None },
            Err(_) => Self {
                error: Some("saved Notion catalog requires review"),
                ..Self::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_cache_does_not_create_or_connect() {
        let tmp = tempfile::tempdir().unwrap();
        let status = NotionOfflineView::read(tmp.path());
        assert!(status.roots.is_empty());
        assert_eq!(status.error, None);
        assert!(!tmp.path().join("ui-state.sqlite3").exists());
    }

    #[test]
    fn invalid_file_does_not_claim_connected_or_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("ui-state.sqlite3");
        std::fs::write(&file, b"not-sqlite").unwrap();
        let before = std::fs::read(&file).unwrap();
        let status = NotionOfflineView::read(tmp.path());
        assert!(status.roots.is_empty());
        assert!(status.error.is_some());
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
}
