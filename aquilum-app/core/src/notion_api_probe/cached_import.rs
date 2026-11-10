//! Token-free migration of a verified Notion API snapshot to independently
//! owned Akula notes. No network access, cache mutation or implicit File Gate
//! authority. The caller selects a registered Vault and explicitly approves
//! create-only publication through the existing File Gate.
use super::{projection::{prepare_import, ImportPreview}, snapshot::SnapshotStore};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct CachedImportReview {
    pub root: String,
    pub digest: String,
    pub saved_at_ms: i64,
    pub page_count: usize,
    pub graph_link_count: usize,
    pub unresolved_count: usize,
    pub can_publish_notes: bool,
    /// Typed database schemas, media, permissions and views are not migrated yet.
    pub full_notion_migration_complete: bool,
}

/// Opening a verified cached root requires no Notion token or API connection.
/// SnapshotStore checks schema, object counts, raw payload checksum and edges.
pub fn read_verified_cached(
    ui_state_path: &Path,
    root: &str,
) -> Result<(CachedImportReview, ImportPreview), String> {
    let store = SnapshotStore::open_read_only(ui_state_path)?;
    let snapshot = store.load(root)?
        .ok_or("Notion root not cached; connect and refresh through API first")?;
    let info = snapshot.info.clone();
    let preview = prepare_import(&snapshot.into_preview())?;
    let review = CachedImportReview {
        root: info.root,
        digest: info.digest,
        saved_at_ms: info.saved_at_ms,
        page_count: preview.candidates.len(),
        graph_link_count: info.link_count,
        unresolved_count: preview.unresolved.len(),
        can_publish_notes: preview.publish_allowed && !preview.candidates.is_empty()
            && preview.unresolved.is_empty(),
        full_notion_migration_complete: false,
    };
    Ok((review, preview))
}

/// Binds operator consent to an exact verified API snapshot checksum. An
/// intervening refresh can never silently switch the source being imported.
pub fn approved_cached_preview(
    ui_state_path: &Path,
    root: &str,
    expected_digest: &str,
) -> Result<ImportPreview, String> {
    if expected_digest.len() != 64
        || !expected_digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("expected a 64-character Notion snapshot digest from --cached-preview".into());
    }
    let (review, preview) = read_verified_cached(ui_state_path, root)?;
    if review.digest != expected_digest {
        return Err("Notion snapshot changed after preview; review the new digest first".into());
    }
    if !review.can_publish_notes {
        return Err("Notion root is incomplete or contains no publishable notes".into());
    }
    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app_core::{Core, CoreEvent},
        files::gate,
        history::Source,
        notion_api_probe::{publish::publish_approved, Link, RawObject, SourcePreview, API_VERSION},
    };
    use serde_json::json;
    use std::{fs, sync::Arc};

    const A: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

    fn source(body: &str) -> SourcePreview {
        SourcePreview {
            format: "synthetic_api",
            api_version: API_VERSION,
            root: A.into(),
            workspace_complete: false,
            root_traversal_complete: true,
            requests: 4,
            objects: vec![
                RawObject {
                    kind: "page".into(), id: A.into(),
                    json: json!({"object":"page","id":A,"properties":{
                        "Name":{"type":"title","title":[{"plain_text":"First"}]}
                    }}),
                },
                RawObject {
                    kind: "page".into(), id: B.into(),
                    json: json!({"object":"page","id":B,"properties":{
                        "Name":{"type":"title","title":[{"plain_text":"Second"}]}
                    }}),
                },
                RawObject {
                    kind: "markdown".into(), id: A.into(),
                    json: json!({"markdown":body}),
                },
                RawObject {
                    kind: "markdown".into(), id: B.into(),
                    json: json!({"markdown":"Second body"}),
                },
            ],
            links: vec![Link {
                from:A.into(), to:B.into(), kind:"relation".into(),
                property_id: Some("test-relation".into()),
            }],
            unresolved_target_ids: vec![],
            warnings: vec![],
        }
    }

    #[test]
    fn api_snapshot_to_independent_gate_notes_without_token_or_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let workspace = dir.path().join("vault");
        fs::create_dir(&app).unwrap();
        fs::create_dir(&workspace).unwrap();
        let path = app.join("ui-state.sqlite3");
        let digest = {
            let mut writer = SnapshotStore::open(&path).unwrap();
            writer.publish(&source("First from Notion"), 123).unwrap().digest
        };
        let (review, _) = read_verified_cached(&path, A).unwrap();
        assert_eq!(review.digest, digest);
        assert!(review.can_publish_notes);
        assert!(!review.full_notion_migration_complete);
        assert_eq!(review.page_count, 2);
        assert_eq!(review.graph_link_count, 1);
        assert_eq!(review.unresolved_count, 0);
        let json = serde_json::to_string(&review).unwrap();
        assert!(!json.contains("First from Notion"));

        let preview = approved_cached_preview(&path, A, &digest).unwrap();
        let core = Core::open(&app, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(workspace.to_str().unwrap(), 1).unwrap();
        assert!(publish_approved(&core, &workspace, &preview, false).is_err());
        assert!(!workspace.join("Notion-import").exists());
        let first = publish_approved(&core, &workspace, &preview, true).unwrap();
        assert!(first.completed);
        assert_eq!(first.created.len(), 2);
        let p = workspace.join(format!("Notion-import/{A}.md"));
        assert!(fs::read_to_string(&p).unwrap().contains("First from Notion"));
        let second = publish_approved(&core, &workspace, &preview, true).unwrap();
        assert!(second.completed);
        assert_eq!(second.existing_untouched.len(), 2);
        gate::write(&core, &p, "local edit", None, Source::Me, None).unwrap();
        let third = publish_approved(&core, &workspace, &preview, true).unwrap();
        assert!(!third.completed);
        assert_eq!(third.conflicts_untouched.len(), 1);
        assert_eq!(fs::read_to_string(&p).unwrap(), "local edit");
    }

    #[test]
    fn exact_digest_and_cache_corruption_fail_without_creating_files() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        fs::create_dir(&app).unwrap();
        let path = app.join("ui-state.sqlite3");
        let initial = {
            let mut writer = SnapshotStore::open(&path).unwrap();
            writer.publish(&source("First"), 123).unwrap().digest
        };
        assert!(approved_cached_preview(&path, A, "invalid").is_err());
        assert!(approved_cached_preview(&path, B, &initial).is_err());
        assert!(approved_cached_preview(&path, A, &"0".repeat(64)).is_err());
        {
            let mut writer = SnapshotStore::open(&path).unwrap();
            writer.publish(&source("Changed"), 124).unwrap();
        }
        assert!(approved_cached_preview(&path, A, &initial)
            .unwrap_err().contains("changed after preview"));
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute("UPDATE notion_cache_objects SET raw_json = ?1
            WHERE root_id = ?2 AND kind = 'markdown' AND object_id = ?3",
            rusqlite::params![r#"{"markdown":"injected"}"#, A, A]).unwrap();
        assert!(read_verified_cached(&path, A).is_err());
        assert!(!app.join("Notion-import").exists());
    }

    #[test]
    fn missing_cache_is_read_only_and_does_not_require_network_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui-state.sqlite3");
        assert!(read_verified_cached(&path, A).is_err());
        assert!(!path.exists());
    }
}
