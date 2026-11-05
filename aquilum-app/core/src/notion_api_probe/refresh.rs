//! Bounded, explicitly invoked Notion-to-local refresh boundary.
//!
//! No background worker, no file gate writes, no token storage, no remote writes.
//! Failed HTTP reads and incomplete previews never replace the last cached root.
use super::{inspect_root, NotionReadTransport, ProbeLimits};
use super::snapshot::{SnapshotInfo, SnapshotStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshChange {
    FirstCopy,
    Changed,
    Unchanged,
}

#[derive(Clone, Debug)]
pub struct RefreshReceipt {
    pub snapshot: SnapshotInfo,
    pub change: RefreshChange,
    pub requests: usize,
}

/// The same entry point can be called from a future UI button or sync worker.
/// The network read and validation finish BEFORE the SQLite replacement begins.
pub fn refresh_root<T: NotionReadTransport>(
    transport: &mut T,
    store: &mut SnapshotStore,
    root: &str,
    limits: ProbeLimits,
    now_ms: i64,
) -> Result<RefreshReceipt, String> {
    let old = store.status(root)?;
    let new = inspect_root(transport, root, limits)?;
    let info = store.publish(&new, now_ms)?;
    let change = match old {
        None => RefreshChange::FirstCopy,
        Some(old) if old.digest == info.digest => RefreshChange::Unchanged,
        Some(_) => RefreshChange::Changed,
    };
    Ok(RefreshReceipt { snapshot: info, change, requests: new.requests })
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::ReadRequest;
    use serde_json::{json, Value};

    const ROOT: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

    #[derive(Clone, Copy)]
    enum ReplyMode { Good, NetworkError, TruncatedMarkdown, UnknownBlocks }

    struct FakeNotion {
        mode: ReplyMode,
        body: String,
        reads: usize,
    }
    impl FakeNotion {
        fn new(body: &str, mode: ReplyMode) -> Self {
            Self { mode, body: body.into(), reads: 0 }
        }
    }
    impl NotionReadTransport for FakeNotion {
        fn read(&mut self, request: &ReadRequest) -> Result<Value, String> {
            self.reads += 1;
            match request {
                ReadRequest::Page(id) => Ok(json!({
                    "object": "page", "id": id, "properties": {
                        "Name": { "type": "title", "title": [{"plain_text": "Test page"}] }
                    }
                })),
                ReadRequest::Markdown(id) => match self.mode {
                    ReplyMode::NetworkError => Err("synthetic HTTP outage".into()),
                    ReplyMode::Good => Ok(json!({
                        "object": "page_markdown", "id": id, "markdown": self.body,
                        "truncated": false, "unknown_block_ids": []
                    })),
                    ReplyMode::TruncatedMarkdown => Ok(json!({
                        "object": "page_markdown", "id": id, "markdown": self.body,
                        "truncated": true, "unknown_block_ids": []
                    })),
                    ReplyMode::UnknownBlocks => Ok(json!({
                        "object": "page_markdown", "id": id, "markdown": self.body,
                        "truncated": false, "unknown_block_ids": [ROOT]
                    })),
                },
                ReadRequest::BlockChildren { id, cursor } if id == ROOT && cursor.is_none() =>
                    Ok(json!({
                        "object": "list", "results": [],
                        "has_more": false, "next_cursor": null
                    })),
                _ => Err("unexpected Notion request in synthetic read-only API".into()),
            }
        }
    }

    #[test]
    fn first_fetch_refresh_no_change_and_offline_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ui-state.sqlite3");
        let mut store = SnapshotStore::open(&path).unwrap();
        let mut api = FakeNotion::new("original", ReplyMode::Good);
        let first = refresh_root(&mut api, &mut store, ROOT, ProbeLimits::default(), 1).unwrap();
        assert_eq!(first.change, RefreshChange::FirstCopy);
        assert_eq!(first.requests, 3);
        let second = refresh_root(&mut api, &mut store, ROOT, ProbeLimits::default(), 2).unwrap();
        assert_eq!(second.change, RefreshChange::Unchanged);
        assert_eq!(first.snapshot.digest, second.snapshot.digest);
        api.body = "updated by Notion".into();
        let third = refresh_root(&mut api, &mut store, ROOT, ProbeLimits::default(), 3).unwrap();
        assert_eq!(third.change, RefreshChange::Changed);
        assert_ne!(second.snapshot.digest, third.snapshot.digest);
        drop(store);
        let offline = SnapshotStore::open_read_only(&path).unwrap()
            .load(ROOT).unwrap().unwrap().into_preview();
        let note = super::super::projection::prepare_import(&offline).unwrap();
        assert_eq!(note.candidates.len(), 1);
        assert!(note.candidates[0].markdown.contains("updated by Notion"));
    }

    #[test]
    fn failed_remote_request_and_partial_notions_never_replace_verified_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ui-state.sqlite3");
        let mut store = SnapshotStore::open(&path).unwrap();
        let first = refresh_root(&mut FakeNotion::new("good", ReplyMode::Good),
            &mut store, ROOT, ProbeLimits::default(), 1).unwrap();
        for bad in [ReplyMode::NetworkError, ReplyMode::TruncatedMarkdown, ReplyMode::UnknownBlocks] {
            let mut api = FakeNotion::new("incomplete", bad);
            assert!(refresh_root(&mut api, &mut store, ROOT, ProbeLimits::default(), 2).is_err());
            let preserved = store.load(ROOT).unwrap().unwrap();
            assert_eq!(preserved.info, first.snapshot);
            assert_eq!(preserved.info.object_count, 2);
        }
    }

    #[test]
    fn denied_probe_budget_cannot_create_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("ui-state.sqlite3");
        let mut store = SnapshotStore::open(&path).unwrap();
        let mut api = FakeNotion::new("sample", ReplyMode::Good);
        let too_small = ProbeLimits {
            max_requests: 1,
            max_objects: 20,
            max_depth: 4,
            max_pages_per_collection: 10,
        };
        assert!(refresh_root(&mut api, &mut store, ROOT, too_small, 1).is_err());
        assert!(store.status(ROOT).unwrap().is_none());
    }
}
