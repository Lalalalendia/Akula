//! Read-only reverse document identity lookup for trusted host callers.
//!
//! Stored document IDs never grant file access. This only returns an active,
//! bounded, normalized path *relative to the specified workspace*. The native
//! host still must verify workspace ownership, canonical paths, read grant,
//! complete current note contents and revision before serving Language cards.
//! Lookup never creates or resurrects a document and never opens a file.

use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use super::database::UiStateDatabase;
use super::error::UiStateError;
use super::paths::normalize_relative;
use super::service::UiStateService;

impl UiStateDatabase {
    pub fn active_document_relative(
        &self,
        workspace_id: Uuid,
        document_id: Uuid,
    ) -> Result<Option<String>, UiStateError> {
        let stored: Option<String> = self.connection.query_row(
            "SELECT relative_path FROM documents
             WHERE id = ?1 AND workspace_id = ?2 AND status = 'active'",
            params![document_id.to_string(), workspace_id.to_string()],
            |row| row.get(0),
        ).optional()?;
        stored.map(|path| normalize_relative(&path)).transpose()
    }
}

impl UiStateService {
    /// Non-authorizing identity lookup: never use the returned relative path
    /// without a fresh workspace-root and file-read verification by the host.
    pub fn active_document_relative(
        &self,
        workspace_id: Uuid,
        document_id: Uuid,
    ) -> Result<Option<String>, UiStateError> {
        self.with_database(|db| db.active_document_relative(workspace_id, document_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_lookup_uses_workspace_and_active_status_without_creating_rows() {
        let mut db = UiStateDatabase::memory().unwrap();
        let a = db.resolve_workspace("vault-A", 1).unwrap();
        let b = db.resolve_workspace("vault-B", 1).unwrap();
        let doc = db.resolve_document(a, "lessons/words.md").unwrap();

        assert_eq!(db.active_document_relative(a, doc).unwrap(),
            Some("lessons/words.md".into()));
        assert_eq!(db.active_document_relative(b, doc).unwrap(), None);
        assert_eq!(db.active_document_relative(a, Uuid::new_v4()).unwrap(), None);

        let before: i64 = db.connection.query_row(
            "SELECT COUNT(*) FROM documents", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(before, 1);

        db.rename_document(doc, "lessons/renamed.md").unwrap();
        assert_eq!(db.active_document_relative(a, doc).unwrap(),
            Some("lessons/renamed.md".into()));

        db.mark_document_missing(doc, 123).unwrap();
        assert_eq!(db.active_document_relative(a, doc).unwrap(), None);
        let after: i64 = db.connection.query_row(
            "SELECT COUNT(*) FROM documents", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(after, before, "read-only lookup must not mint a new ID");
    }

    #[test]
    fn stored_parent_escape_and_absolute_paths_fail_closed() {
        let mut db = UiStateDatabase::memory().unwrap();
        let workspace = db.resolve_workspace("workspace", 1).unwrap();
        for path in ["../secret.md", "/elsewhere/private.md", "folder/../../a.md"] {
            let id = db.resolve_document(workspace, path).unwrap();
            assert!(matches!(
                db.active_document_relative(workspace, id),
                Err(UiStateError::InvalidInput { .. })
            ), "unsafe stored path should be rejected: {path}");
        }
    }

    #[test]
    fn public_service_has_same_read_only_non_authorizing_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let ui = UiStateService::open(&dir.path().join("ui-state.sqlite"));
        let workspace = ui.resolve_workspace("hosted-vault", 1).unwrap();
        let document = ui.resolve_document(workspace, "grammar/table.md").unwrap();
        assert_eq!(ui.active_document_relative(workspace, document).unwrap(),
            Some("grammar/table.md".into()));
        assert_eq!(ui.active_document_relative(Uuid::new_v4(), document).unwrap(), None);
        ui.mark_document_missing(document, 4).unwrap();
        assert_eq!(ui.active_document_relative(workspace, document).unwrap(), None);
    }
}
