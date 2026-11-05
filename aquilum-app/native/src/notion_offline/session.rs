//! User-invoked offline reader for one committed Notion source.
//! Heavy checksum verification and projection run on a named worker,
//! never on the native window's event thread.
use aquilum_core::notion_api_probe::projection::{prepare_import, ImportCandidate};
use aquilum_core::notion_api_probe::snapshot::SnapshotStore;
use std::path::PathBuf;
use std::sync::{Arc, mpsc::{self, Receiver, Sender}};

pub const PAGE_BATCH: usize = 12;
pub const TEXT_CHARS: usize = 2_400;
const MAX_PAGES: usize = 4_096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState { Idle, Loading, Ready, NeedsReview }

#[derive(Clone, Debug)]
pub struct PageEntry { pub id: String, pub title: String }

#[derive(Clone, Debug)]
pub struct PageText {
    pub id: String,
    pub title: String,
    pub body: String,
    pub start_char: usize,
    pub more_after: bool,
    pub more_before: bool,
}

#[derive(Clone, Debug)]
pub struct SelectionView {
    pub state: LoadState,
    pub root: Option<String>,
    pub displayed_pages: Vec<PageEntry>,
    pub page_start: usize,
    pub page_count: usize,
    pub chosen: Option<PageText>,
}

type Response = (u64, String, Result<Vec<ImportCandidate>, ()>);

pub struct NotionSession {
    generation: u64,
    root: Option<String>,
    state: LoadState,
    pages: Vec<ImportCandidate>,
    page_start: usize,
    chosen: Option<String>,
    text_start: usize,
    tx: Sender<Response>,
    rx: Receiver<Response>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl NotionSession {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            generation: 0, root: None, state: LoadState::Idle,
            pages: Vec::new(), page_start: 0, chosen: None,
            text_start: 0, tx, rx, wake,
        }
    }

    /// Calling twice cancels the previous result by generation. No remote I/O.
    pub fn open(&mut self, data_dir: PathBuf, root: String) {
        self.generation = self.generation.wrapping_add(1);
        let revision = self.generation;
        self.root = Some(root.clone());
        self.state = LoadState::Loading;
        self.pages.clear();
        self.chosen = None;
        self.page_start = 0;
        self.text_start = 0;
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        let spawn = std::thread::Builder::new()
            .name("akula-notion-offline-read".into()).spawn(move || {
                let result = verified_pages(data_dir, &root);
                let _ = tx.send((revision, root, result));
                wake();
            });
        if spawn.is_err() {
            self.state = LoadState::NeedsReview;
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok((generation, root, outcome)) = self.rx.try_recv() {
            if generation != self.generation || self.root.as_deref() != Some(&root) {
                continue;
            }
            match outcome {
                Ok(pages) => {
                    self.pages = pages;
                    self.state = LoadState::Ready;
                }
                Err(()) => {
                    self.pages.clear();
                    self.state = LoadState::NeedsReview;
                }
            }
            changed = true;
        }
        changed
    }

    pub fn page(&mut self, id: &str) -> bool {
        if self.state != LoadState::Ready ||
            !self.pages.iter().any(|p| p.source_id == id) { return false; }
        self.chosen = Some(id.into());
        self.text_start = 0;
        true
    }

    pub fn page_batch(&mut self, first: usize) -> bool {
        if self.state != LoadState::Ready || first >= self.pages.len() ||
            first % PAGE_BATCH != 0 { return false; }
        self.page_start = first;
        self.chosen = None;
        self.text_start = 0;
        true
    }

    pub fn text_chunk(&mut self, start: usize) -> bool {
        let Some(current) = self.pages.iter().find(|p|
            Some(p.source_id.as_str()) == self.chosen.as_deref()) else { return false };
        let len = current.markdown.chars().count();
        if start >= len || start % TEXT_CHARS != 0 { return false; }
        self.text_start = start;
        true
    }

    pub fn view(&self) -> SelectionView {
        let displayed_pages = self.pages.iter()
            .skip(self.page_start).take(PAGE_BATCH)
            .map(|page| PageEntry {
                id: page.source_id.clone(),
                title: first_line_title(&page.markdown),
            }).collect();
        let chosen = self.chosen.as_ref().and_then(|id| {
            let p = self.pages.iter().find(|p| &p.source_id == id)?;
            let content = p.markdown.chars().skip(self.text_start)
                .take(TEXT_CHARS + 1).collect::<String>();
            let more_after = content.chars().count() > TEXT_CHARS;
            let body = content.chars().take(TEXT_CHARS).collect();
            Some(PageText {
                id: id.clone(), title: first_line_title(&p.markdown), body,
                start_char: self.text_start, more_after,
                more_before: self.text_start > 0,
            })
        });
        SelectionView {
            state: self.state.clone(), root: self.root.clone(), displayed_pages,
            page_start: self.page_start, page_count: self.pages.len(), chosen,
        }
    }
}

fn first_line_title(markdown: &str) -> String {
    let line = markdown.lines().next().unwrap_or("(untitled page)")
        .trim_start_matches("# ");
    let name = line.chars().filter(|c| !c.is_control())
        .take(96).collect::<String>();
    if name.is_empty() { "(untitled page)".into() } else { name }
}

fn verified_pages(data_dir: PathBuf, root: &str) -> Result<Vec<ImportCandidate>, ()> {
    // Opening the SQLite file cannot create a cache, even on a failed read.
    let store = SnapshotStore::open_read_only(&data_dir.join("ui-state.sqlite3"))
        .map_err(|_| ())?;
    let loaded = store.load(root).map_err(|_| ())?.ok_or(())?;
    let preview = prepare_import(&loaded.into_preview()).map_err(|_| ())?;
    if !preview.publish_allowed || preview.candidates.len() > MAX_PAGES {
        return Err(());
    }
    let mut pages = preview.candidates;
    pages.sort_by(|a, b| {
        first_line_title(&a.markdown).cmp(&first_line_title(&b.markdown))
            .then_with(|| a.source_id.cmp(&b.source_id))
    });
    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::notion_api_probe::{RawObject, SourcePreview, API_VERSION};
    use serde_json::json;
    use std::time::Duration;

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn fixture() -> SourcePreview {
        SourcePreview {
            format: "test", api_version: API_VERSION, root: A.into(),
            workspace_complete: false, root_traversal_complete: true, requests: 2,
            objects: vec![
                RawObject { kind: "page".into(), id: A.into(),
                    json: json!({"properties":{"Name":{"type":"title",
                        "title":[{"plain_text":"Alpha"}]}}}) },
                RawObject { kind: "markdown".into(), id: A.into(),
                    json: json!({"markdown": format!("{}\n", "a".repeat(TEXT_CHARS+40))}) },
                RawObject { kind: "page".into(), id: B.into(),
                    json: json!({"properties":{"Name":{"type":"title",
                        "title":[{"plain_text":"Beta"}]}}}) },
                RawObject { kind: "markdown".into(), id: B.into(),
                    json: json!({"markdown":"Beta content"}) },
            ],
            links: Vec::new(), unresolved_target_ids: Vec::new(), warnings: Vec::new(),
        }
    }

    #[test]
    fn full_root_integrity_verified_before_any_page_is_visible() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-state.sqlite3");
        let mut writer = SnapshotStore::open(&path).unwrap();
        writer.publish(&fixture(), 10).unwrap();
        drop(writer);
        let mut session = NotionSession::new(Arc::new(|| {}));
        session.open(dir.path().into(), A.into());
        assert_eq!(session.view().state, LoadState::Loading);
        for _ in 0..100 {
            if session.poll() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(session.view().state, LoadState::Ready);
        assert_eq!(session.view().page_count, 2);
        assert!(session.page(A));
        let page = session.view().chosen.unwrap();
        assert!(page.body.contains("Alpha"));
        assert!(page.more_after);
        assert!(session.text_chunk(TEXT_CHARS));
        let after = session.view().chosen.unwrap();
        assert!(after.more_before);
        assert_ne!(page.body, after.body);
        assert!(!session.page("cccccccc-cccc-4ccc-8ccc-cccccccccccc"));
    }

    #[test]
    fn missing_or_corrupt_root_never_displays_unverified_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = NotionSession::new(Arc::new(|| {}));
        session.open(dir.path().into(), A.into());
        for _ in 0..100 {
            if session.poll() { break; }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(session.view().state, LoadState::NeedsReview);
        assert!(!dir.path().join("ui-state.sqlite3").exists());
        assert!(session.view().chosen.is_none());
    }

    #[test]
    fn forged_and_stale_worker_responses_cannot_replace_current_selection() {
        let mut session = NotionSession::new(Arc::new(|| {}));
        session.open(PathBuf::from("missing"), A.into());
        session.open(PathBuf::from("missing"), B.into());
        session.tx.send((session.generation - 1, A.into(), Ok(vec![
            ImportCandidate { source_id: A.into(), relative_path: String::new(),
                markdown: "# Injected".into(), relations: Vec::new() }
        ]))).unwrap();
        session.poll();
        assert_eq!(session.view().root.as_deref(), Some(B));
        assert_ne!(session.view().state, LoadState::Ready);
    }
}
