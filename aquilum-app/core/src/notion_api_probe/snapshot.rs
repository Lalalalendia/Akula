//! Versioned local copy of a complete, explicitly selected Notion root.
//!
//! Stored inside Akula's existing ui-state SQLite database, never in the vault.
//! This module does NOT authorize remote writes, local File Gate writes, polling,
//! token storage, or claims about pages outside the selected root.
use super::{Link, RawObject, SourcePreview, API_VERSION};
use crate::ui_state::database::UiStateDatabase;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use uuid::Uuid;

const MAX_OBJECTS: usize = 20_000;
const MAX_LINKS: usize = 320_000;
const MAX_OBJECT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotInfo {
    pub root: String,
    pub generation: String,
    pub api_version: String,
    pub digest: String,
    pub saved_at_ms: i64,
    pub object_count: usize,
    pub link_count: usize,
}

pub struct CachedSnapshot {
    pub info: SnapshotInfo,
    pub objects: Vec<RawObject>,
    pub links: Vec<Link>,
}

impl CachedSnapshot {
    /// Only the selected root is known complete, never the entire workspace.
    pub fn into_preview(self) -> SourcePreview {
        SourcePreview {
            format: "akula_notion_local_root_cache_01",
            api_version: API_VERSION,
            root: self.info.root,
            workspace_complete: false,
            root_traversal_complete: true,
            requests: 0,
            objects: self.objects,
            links: self.links,
            unresolved_target_ids: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub struct SnapshotStore {
    database: UiStateDatabase,
}

impl SnapshotStore {
    /// The path is Akula appdata/ui-state.sqlite3, not a user-controlled vault.
    pub fn open(ui_state_path: &Path) -> Result<Self, String> {
        let database = UiStateDatabase::open(ui_state_path)
            .map_err(|e| format!("cannot open Akula UI state: {e:?}"))?;
        // A committed, acknowledged snapshot must survive a power loss.
        database.connection.execute_batch("PRAGMA synchronous = FULL;")
            .map_err(|_| "cannot enable durable Notion snapshot writes")?;
        Ok(Self { database })
    }

    /// Open a committed cache without creating or migrating any UI-state file.
    /// The offline reader requires no token, database writer or network.
    pub fn open_read_only(ui_state_path: &Path) -> Result<Self, String> {
        let meta = std::fs::symlink_metadata(ui_state_path)
            .map_err(|_| "Notion root not cached; local SQLite file is missing")?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("Notion cache path must be an existing regular file".into());
        }
        let connection = rusqlite::Connection::open_with_flags(
            ui_state_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).map_err(|_| "cannot open existing Notion cache read-only")?;
        connection.execute_batch("PRAGMA query_only = ON;")
            .map_err(|_| "cannot protect offline Notion cache connection")?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|_| "cannot read offline Notion cache schema")?;
        if version != 9 {
            return Err("offline Notion cache requires a verified SQLite v9 schema".into());
        }
        Ok(Self { database: UiStateDatabase { connection } })
    }

    /// Read-only offline metadata, with no Notion token or network connection.
    pub fn status(&self, root: &str) -> Result<Option<SnapshotInfo>, String> {
        let root = canonical_root(root)?;
        let row = self.database.connection.query_row(
            "SELECT generation, api_version, checksum, saved_at_ms, object_count, link_count
             FROM notion_cache_sources WHERE root_id = ?1",
            [&root],
            |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            )),
        ).optional().map_err(|_| "cannot read Notion snapshot metadata")?;
        let Some((generation, api_version, digest, saved_at_ms, objects, links)) = row else {
            return Ok(None);
        };
        Ok(Some(SnapshotInfo {
            root, generation, api_version, digest, saved_at_ms,
            object_count: usize::try_from(objects).map_err(|_| "invalid Notion object count")?,
            link_count: usize::try_from(links).map_err(|_| "invalid Notion link count")?,
        }))
    }

    /// Do not serve a partial or corrupt cached copy as a complete snapshot.
    pub fn load(&self, root: &str) -> Result<Option<CachedSnapshot>, String> {
        let Some(info) = self.status(root)? else { return Ok(None) };
        if info.api_version != API_VERSION {
            return Err("Notion cache API version needs an explicit migration".into());
        }
        if info.object_count > MAX_OBJECTS || info.link_count > MAX_LINKS {
            return Err("Notion cache exceeds reader safety budget".into());
        }
        let mut objects = Vec::new();
        {
            let mut query = self.database.connection.prepare(
                "SELECT kind, object_id, raw_json FROM notion_cache_objects
                 WHERE root_id = ?1 ORDER BY kind, object_id",
            ).map_err(|_| "cannot prepare Notion cache object reader")?;
            let rows = query.query_map([&info.root], |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))).map_err(|_| "cannot query Notion cache objects")?;
            for result in rows {
                let (kind, id, raw) = result.map_err(|_| "invalid Notion cache object row")?;
                let json: Value = serde_json::from_str(&raw)
                    .map_err(|_| "invalid Notion cache JSON")?;
                objects.push(RawObject { kind, id, json });
                if objects.len() > MAX_OBJECTS {
                    return Err("Notion cache contains too many objects".into());
                }
            }
        }
        let mut links = Vec::new();
        {
            let mut query = self.database.connection.prepare(
                "SELECT source_id, target_id, kind, property_id FROM notion_cache_edges
                 WHERE root_id = ?1 ORDER BY source_id, target_id, kind, property_id",
            ).map_err(|_| "cannot prepare Notion link reader")?;
            let rows = query.query_map([&info.root], |row| Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))).map_err(|_| "cannot query Notion cache links")?;
            for result in rows {
                let (from, to, kind, property) =
                    result.map_err(|_| "invalid Notion cache edge")?;
                links.push(Link {
                    from, to, kind,
                    property_id: if property.is_empty() { None } else { Some(property) },
                });
                if links.len() > MAX_LINKS {
                    return Err("Notion cache contains too many links".into());
                }
            }
        }
        if objects.len() != info.object_count || links.len() != info.link_count {
            return Err("Notion cache counts do not match committed snapshot".into());
        }
        if fingerprint(&info.root, &info.api_version, &objects, &links)?
            != info.digest
        {
            return Err("Notion cache checksum mismatch; retaining last source without claiming validity".into());
        }
        Ok(Some(CachedSnapshot { info, objects, links }))
    }

    /// Apply a complete root snapshot as a single SQLite transaction.
    /// If ANY SQL write fails, the previous committed copy remains visible.
    pub fn publish(&mut self, source: &SourcePreview, now_ms: i64)
        -> Result<SnapshotInfo, String>
    {
        self.publish_inner(source, now_ms, None)
    }

    fn publish_inner(&mut self, source: &SourcePreview, now_ms: i64,
        fail_after: Option<usize>) -> Result<SnapshotInfo, String>
    {
        validate_source(source)?;
        if now_ms < 0 { return Err("negative snapshot timestamp".into()); }
        let digest = fingerprint(&source.root, source.api_version,
            &source.objects, &source.links)?;
        let generation = Uuid::new_v4().to_string();
        let object_count = i64::try_from(source.objects.len())
            .map_err(|_| "too many Notion objects")?;
        let link_count = i64::try_from(source.links.len())
            .map_err(|_| "too many Notion links")?;
        let tx = self.database.connection.transaction()
            .map_err(|_| "cannot start atomic Notion snapshot update")?;
        tx.execute(
            "INSERT INTO notion_cache_sources
                 (root_id, generation, api_version, checksum, saved_at_ms, object_count, link_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(root_id) DO UPDATE SET
                 generation=excluded.generation, api_version=excluded.api_version,
                 checksum=excluded.checksum, saved_at_ms=excluded.saved_at_ms,
                 object_count=excluded.object_count, link_count=excluded.link_count",
            params![source.root, generation, source.api_version, digest,
                now_ms, object_count, link_count],
        ).map_err(|_| "cannot stage Notion snapshot header")?;
        tx.execute("DELETE FROM notion_cache_objects WHERE root_id = ?1", [&source.root])
            .map_err(|_| "cannot stage Notion object replacement")?;
        tx.execute("DELETE FROM notion_cache_edges WHERE root_id = ?1", [&source.root])
            .map_err(|_| "cannot stage Notion edge replacement")?;

        for (index, object) in source.objects.iter().enumerate() {
            let json = serde_json::to_string(&object.json)
                .map_err(|_| "cannot encode Notion object")?;
            tx.execute(
                "INSERT INTO notion_cache_objects (root_id, kind, object_id, raw_json)
                 VALUES (?1, ?2, ?3, ?4)",
                params![source.root, object.kind, object.id, json],
            ).map_err(|_| "cannot stage Notion object")?;
            if fail_after == Some(index + 1) {
                return Err("test interruption before snapshot commit".into());
            }
        }
        for link in &source.links {
            tx.execute(
                "INSERT INTO notion_cache_edges
                     (root_id, source_id, target_id, kind, property_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![source.root, link.from, link.to, link.kind,
                    link.property_id.as_deref().unwrap_or("")],
            ).map_err(|_| "cannot stage Notion edge")?;
        }
        tx.commit().map_err(|_| "atomic Notion snapshot commit failed")?;
        Ok(SnapshotInfo {
            root: source.root.clone(), generation, api_version: source.api_version.into(),
            digest, saved_at_ms: now_ms,
            object_count: source.objects.len(), link_count: source.links.len(),
        })
    }
}

fn canonical_root(root: &str) -> Result<String, String> {
    let id = Uuid::parse_str(root).map_err(|_| "invalid Notion root UUID")?.to_string();
    if id != root { return Err("Notion root UUID must be canonical".into()); }
    Ok(id)
}

fn validate_source(source: &SourcePreview) -> Result<(), String> {
    canonical_root(&source.root)?;
    if source.api_version != API_VERSION {
        return Err("unexpected Notion API version".into());
    }
    if !source.root_traversal_complete || !source.warnings.is_empty()
        || !source.unresolved_target_ids.is_empty()
    {
        return Err("incomplete Notion root cannot replace last verified copy".into());
    }
    if source.objects.is_empty() || source.objects.len() > MAX_OBJECTS
        || source.links.len() > MAX_LINKS
    {
        return Err("Notion snapshot object/link limit exceeded".into());
    }
    let mut known_keys = BTreeSet::new();
    let mut known_ids = BTreeSet::new();
    let mut markdown_ids = BTreeSet::new();
    let mut page_ids = BTreeSet::new();
    let mut bytes = 0usize;
    for obj in &source.objects {
        if obj.kind.is_empty() || obj.kind.len() > 64
            || !obj.kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            || obj.id.is_empty() || obj.id.len() > 512
            || obj.id.chars().any(char::is_control)
        {
            return Err("invalid Notion cached object key".into());
        }
        if !known_keys.insert((&obj.kind, &obj.id)) {
            return Err("duplicate Notion kind/ID in source snapshot".into());
        }
        let encoded = serde_json::to_vec(&obj.json)
            .map_err(|_| "cannot serialize Notion raw JSON")?;
        if encoded.len() > MAX_OBJECT_BYTES {
            return Err("Notion raw object exceeds snapshot byte limit".into());
        }
        bytes = bytes.checked_add(encoded.len())
            .ok_or("Notion source size overflow")?;
        if bytes > MAX_SNAPSHOT_BYTES {
            return Err("Notion root exceeds local snapshot byte limit".into());
        }
        if obj.kind == "page" { page_ids.insert(obj.id.as_str()); }
        if obj.kind == "markdown" { markdown_ids.insert(obj.id.as_str()); }
        if obj.kind != "property_item" {
            if Uuid::parse_str(&obj.id).map(|id| id.to_string()).ok().as_deref()
                != Some(obj.id.as_str())
            {
                return Err("invalid Notion object UUID".into());
            }
            known_ids.insert(obj.id.as_str());
        }
    }
    if !page_ids.contains(source.root.as_str()) || !page_ids.is_subset(&markdown_ids) {
        return Err("Notion root or page Markdown is missing".into());
    }
    let mut unique_links = BTreeSet::new();
    for edge in &source.links {
        if !unique_links.insert(edge) { return Err("duplicate Notion graph edge".into()); }
        if !known_ids.contains(edge.from.as_str()) || !known_ids.contains(edge.to.as_str()) {
            return Err("Notion graph contains an unresolved source/target".into());
        }
        if edge.kind.is_empty() || edge.kind.len() > 64
            || !edge.kind.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            || edge.property_id.as_deref().is_some_and(|id| {
                id.is_empty() || id.len() > 512 || id.chars().any(char::is_control)
            })
        {
            return Err("invalid Notion graph edge metadata".into());
        }
    }
    Ok(())
}

fn fingerprint(root: &str, version: &str, objects: &[RawObject], links: &[Link])
    -> Result<String, String>
{
    fn absorb(h: &mut blake3::Hasher, bytes: &[u8]) {
        h.update(&(bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    }
    let mut h = blake3::Hasher::new();
    absorb(&mut h, b"akula-notion-snapshot-v1");
    absorb(&mut h, root.as_bytes());
    absorb(&mut h, version.as_bytes());
    let mut objects: Vec<&RawObject> = objects.iter().collect();
    objects.sort_by(|a, b| (&a.kind, &a.id).cmp(&(&b.kind, &b.id)));
    for object in objects {
        absorb(&mut h, object.kind.as_bytes());
        absorb(&mut h, object.id.as_bytes());
        let raw = serde_json::to_vec(&object.json)
            .map_err(|_| "cannot hash Notion JSON")?;
        absorb(&mut h, &raw);
    }
    let mut links: Vec<&Link> = links.iter().collect();
    links.sort();
    for link in links {
        absorb(&mut h, link.from.as_bytes());
        absorb(&mut h, link.to.as_bytes());
        absorb(&mut h, link.kind.as_bytes());
        absorb(&mut h, link.property_id.as_deref().unwrap_or("").as_bytes());
    }
    Ok(h.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const C: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    const D: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";

    fn fixture(root: &str) -> SourcePreview {
        let mut objects = Vec::new();
        let ids = if root == A { vec![A, B, C, D] } else { vec![root] };
        for (n, id) in ids.iter().enumerate() {
            objects.push(RawObject { kind: "page".into(), id: (*id).into(),
                json: json!({"object":"page","id":id,
                    "properties":{"Name":{"type":"title",
                        "title":[{"plain_text":format!("Record {n}")}]}}}) });
            objects.push(RawObject { kind: "markdown".into(), id: (*id).into(),
                json: json!({"markdown":format!("Text {n}")}) });
        }
        SourcePreview {
            format: "test", api_version: API_VERSION, root: root.into(),
            workspace_complete: false, root_traversal_complete: true, requests: 4,
            objects, links: if root == A {
                vec![
                    Link { from: A.into(), to: B.into(), kind: "relation".into(),
                        property_id: Some("rel".into()) },
                    Link { from: C.into(), to: D.into(), kind: "relation".into(),
                        property_id: Some("rel".into()) },
                ]
            } else { Vec::new() },
            unresolved_target_ids: Vec::new(), warnings: Vec::new(),
        }
    }

    #[test]
    fn two_bases_four_rows_two_relations_persist_and_open_offline() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("ui-state.sqlite3");
        let first = {
            let mut store = SnapshotStore::open(&db_path).unwrap();
            store.publish(&fixture(A), 123).unwrap()
        };
        let store = SnapshotStore::open(&db_path).unwrap();
        let cached = store.load(A).unwrap().unwrap();
        assert_eq!(cached.info, first);
        assert_eq!(cached.info.object_count, 8);
        assert_eq!(cached.info.link_count, 2);
        let offline = cached.into_preview();
        assert!(offline.root_traversal_complete);
        assert!(!offline.workspace_complete);
        assert_eq!(offline.links.len(), 2);
        let imported = super::super::projection::prepare_import(&offline).unwrap();
        assert!(imported.publish_allowed);
        assert_eq!(imported.candidates.len(), 4);
        assert!(imported.candidates.iter().any(|c| c.markdown.contains(B)));
    }

    #[test]
    fn retry_and_other_root_cannot_duplicate_or_destroy_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SnapshotStore::open(&dir.path().join("ui-state.sqlite3")).unwrap();
        let old = store.publish(&fixture(A), 123).unwrap();
        store.publish(&fixture(B), 124).unwrap();
        let refreshed = store.publish(&fixture(A), 125).unwrap();
        assert_eq!(old.digest, refreshed.digest);
        assert_ne!(old.generation, refreshed.generation);
        assert_eq!(store.load(A).unwrap().unwrap().objects.len(), 8);
        assert_eq!(store.load(B).unwrap().unwrap().objects.len(), 2);
    }

    #[test]
    fn incomplete_or_corrupt_candidate_never_replaces_last_successful_copy() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SnapshotStore::open(&dir.path().join("ui-state.sqlite3")).unwrap();
        let old = store.publish(&fixture(A), 123).unwrap();
        let mut partial = fixture(A);
        partial.root_traversal_complete = false;
        partial.warnings.push("cursor not exhausted".into());
        assert!(store.publish(&partial, 124).is_err());
        let mut duplicate = fixture(A);
        duplicate.objects.push(duplicate.objects[0].clone());
        assert!(store.publish(&duplicate, 124).is_err());
        assert_eq!(store.status(A).unwrap(), Some(old));
    }

    #[test]
    fn interrupted_sql_transaction_rolls_back_old_graph_and_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SnapshotStore::open(&dir.path().join("ui-state.sqlite3")).unwrap();
        let before = store.publish(&fixture(A), 10).unwrap();
        let mut updated = fixture(A);
        updated.objects[1].json = json!({"markdown":"changed"});
        assert!(store.publish_inner(&updated, 20, Some(1)).is_err());
        let cached = store.load(A).unwrap().unwrap();
        assert_eq!(cached.info, before);
        assert_eq!(cached.links.len(), 2);
        assert_eq!(cached.objects.iter().find(|o| o.kind == "markdown" && o.id == A)
            .unwrap().json["markdown"], "Text 0");
    }

    #[test]
    fn offline_cache_open_does_not_create_or_mutate_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-state.sqlite3");
        assert!(SnapshotStore::open_read_only(&path).is_err());
        assert!(!path.exists());
        let mut writable = SnapshotStore::open(&path).unwrap();
        let before = writable.publish(&fixture(A), 100).unwrap();
        drop(writable);
        let read_only = SnapshotStore::open_read_only(&path).unwrap();
        assert_eq!(read_only.load(A).unwrap().unwrap().info, before);
        assert!(read_only.database.connection.execute(
            "DELETE FROM notion_cache_objects WHERE root_id = ?1", [A],
        ).is_err());
        drop(read_only);
        let writable = SnapshotStore::open(&path).unwrap();
        assert_eq!(writable.status(A).unwrap(), Some(before));
    }

    #[test]
    fn successful_refresh_replaces_changed_content_and_links_for_one_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SnapshotStore::open(&dir.path().join("ui-state.sqlite3")).unwrap();
        let previous = store.publish(&fixture(A), 100).unwrap();
        let separate = store.publish(&fixture(B), 100).unwrap();
        let mut update = fixture(A);
        update.objects.iter_mut()
            .find(|o| o.kind == "markdown" && o.id == A).unwrap()
            .json = json!({"markdown":"Changed by Notion"});
        update.links.pop();
        let refreshed = store.publish(&update, 101).unwrap();
        assert_ne!(previous.digest, refreshed.digest);
        assert_eq!(refreshed.link_count, 1);
        assert_eq!(store.load(A).unwrap().unwrap().links.len(), 1);
        let saved = store.load(A).unwrap().unwrap().into_preview();
        assert!(super::super::projection::prepare_import(&saved).unwrap()
            .candidates.iter().any(|c| c.markdown.contains("Changed by Notion")));
        assert_eq!(store.status(B).unwrap(), Some(separate));
    }

    #[test]
    fn corruption_is_detected_before_offline_display() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SnapshotStore::open(&dir.path().join("ui-state.sqlite3")).unwrap();
        store.publish(&fixture(A), 10).unwrap();
        store.database.connection.execute(
            "UPDATE notion_cache_objects SET raw_json = ?1
             WHERE root_id = ?2 AND kind = 'markdown' AND object_id = ?3",
            params![r#"{"markdown":"altered"}"#, A, A],
        ).unwrap();
        assert!(store.load(A).is_err());
    }
}
