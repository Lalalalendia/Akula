//! Conditional workspace path relocation in the UI-state database.
//!
//! This changes *only* the registered path. It deliberately does not rename
//! a directory or authorize a filesystem mutation. A future coordinator must
//! first complete and durably verify a safe on-disk move, then call this method.
use super::*;

impl UiStateDatabase {
    pub fn replace_workspace_path_after_move(
        &mut self,
        workspace_id: Uuid,
        expected_old_path: &str,
        destination_path: &str,
    ) -> Result<(), UiStateError> {
        if expected_old_path.trim().is_empty()
            || destination_path.trim().is_empty()
            || expected_old_path == destination_path
        {
            return Err(UiStateError::InvalidInput {
                message: "workspace paths must be nonempty and different".into(),
            });
        }
        let transaction = self.connection.transaction()?;
        // CAS by both UUID and prior path; never create a second workspace ID
        // or reassign another workspace's documents, tabs or view states.
        let changed = transaction.execute(
            "UPDATE workspaces SET path = ?3
             WHERE id = ?1 AND path = ?2
               AND NOT EXISTS (SELECT 1 FROM workspaces WHERE path = ?3)",
            params![workspace_id.to_string(), expected_old_path, destination_path],
        )?;
        if changed != 1 {
            return Err(UiStateError::InvalidInput {
                message: "workspace changed, disappeared, or destination is already registered".into(),
            });
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_state::tests::{batch, setup};
    use crate::ui_state::models::SaveReaderStateInput;

    #[test]
    fn path_change_preserves_workspace_and_document_session_identity() {
        let (mut db, workspace, document, epoch) = setup();
        assert!(db.save_batch(&batch(workspace, document, epoch, 1, 28)).unwrap());
        db.set_home_page("C:/notes", "index.md").unwrap();
        db.save_reader_state(&SaveReaderStateInput {
            workspace_id: workspace,
            book_file: "Files/book.epub".to_owned(),
            current: 42,
            cfi: Some("epubcfi(/6/2)".to_owned()),
            now_ms: 10,
        }).unwrap();
        db.replace_workspace_path_after_move(workspace, "C:/notes", "C:/notes-renamed").unwrap();
        let known = db.list_workspaces().unwrap();
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].id, workspace);
        assert_eq!(known[0].path, "C:/notes-renamed");
        assert_eq!(known[0].home_page, "index.md");
        assert_eq!(db.resolve_workspace("C:/notes-renamed", 25).unwrap(), workspace);
        assert_eq!(db.resolve_document(workspace, "note.md").unwrap(), document);
        let restored = db.load_session(workspace, "main").unwrap();
        assert_eq!(restored.tabs.len(), 1);
        assert_eq!(restored.tabs[0].document_id, Some(document));
        assert_eq!(restored.views[0].fallback_anchor, 28);
        let reader = db.load_reader_state(workspace, "Files/book.epub").unwrap().unwrap();
        assert_eq!(reader.current, 42);
        assert_eq!(reader.cfi.as_deref(), Some("epubcfi(/6/2)"));
    }

    #[test]
    fn rejects_stale_old_path_and_registered_destination_without_mutation() {
        let (mut db, workspace, _document, _epoch) = setup();
        let other = db.resolve_workspace("C:/other", 2).unwrap();
        assert!(db.replace_workspace_path_after_move(workspace, "C:/wrong", "C:/new").is_err());
        assert!(db.replace_workspace_path_after_move(workspace, "C:/notes", "C:/other").is_err());
        assert!(db.replace_workspace_path_after_move(workspace, "C:/notes", "").is_err());
        assert!(db.replace_workspace_path_after_move(workspace, "C:/notes", "C:/notes").is_err());
        let list = db.list_workspaces().unwrap();
        assert!(list.iter().any(|w| w.id == workspace && w.path == "C:/notes"));
        assert!(list.iter().any(|w| w.id == other && w.path == "C:/other"));
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn refuses_replaying_a_completed_relocation() {
        let (mut db, workspace, _doc, _epoch) = setup();
        db.replace_workspace_path_after_move(workspace, "C:/notes", "C:/done").unwrap();
        assert!(db.replace_workspace_path_after_move(workspace, "C:/notes", "C:/again").is_err());
        assert_eq!(db.resolve_workspace("C:/done", 3).unwrap(), workspace);
        assert_eq!(db.list_workspaces().unwrap().len(), 1);
    }
}
