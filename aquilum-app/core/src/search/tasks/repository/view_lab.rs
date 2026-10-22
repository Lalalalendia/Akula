//! Experimental host-owned task preview. This is not a guest/plugin API.
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::search::error::SearchError;
use crate::search::models::SearchIndexState;
use crate::search::paths::{canonical_path, is_hidden, is_markdown, strip_root};
use crate::search::service::SearchService;

use super::Task;

const MAX_VISIBLE_PLUS_ONE: i64 = 65;

fn read_preview(db: &Connection, path: &str) -> Result<Vec<Task>, SearchError> {
    let mut statement = db.prepare(
        "SELECT line, done, text FROM note_tasks
         WHERE path = ?1 ORDER BY ordinal LIMIT ?2",
    )?;
    let rows = statement.query_map(rusqlite::params![path, MAX_VISIBLE_PLUS_ONE], |row| {
        Ok(Task {
            line: row.get::<_, i64>(0)? as usize,
            done: row.get::<_, i64>(1)? != 0,
            text: row.get::<_, String>(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

impl SearchService {
    /// Host-only view of a single indexed Markdown note. No filesystem write.
    pub fn task_preview_for_document(
        &self,
        workspace: &str,
        document: &Path,
    ) -> Result<Vec<Task>, SearchError> {
        let document = canonical_path(document);
        self.with_index(workspace, |index| {
            if strip_root(&index.root, &document).is_none()
                || !is_markdown(&document)
                || is_hidden(&index.root, &document)
            {
                return Err(SearchError::invalid_workspace("Document outside the indexed note scope"));
            }
            let before = index.progress.snapshot();
            if before.state != SearchIndexState::Ready || before.updating {
                return Err(SearchError::Unavailable { message: "Index updating".to_owned() });
            }
            let db = Connection::open_with_flags(&index.metadata_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let rows = read_preview(&db, &document.to_string_lossy())?;
            let after = index.progress.snapshot();
            if after.generation != before.generation
                || after.revision != before.revision
                || after.state != SearchIndexState::Ready
                || after.updating
            {
                return Err(SearchError::Unavailable { message: "Index changed during view".to_owned() });
            }
            Ok(rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rusqlite::Connection;

    use super::read_preview;

    #[test]
    fn read_only_preview_is_bounded() {
        let mut db = Connection::open_in_memory().unwrap();
        super::super::open_schema(&db).unwrap();
        let text = (0..80).map(|n| format!("- [ ] task {n}")).collect::<Vec<_>>().join("\n");
        let transaction = db.transaction().unwrap();
        super::super::index_document(&transaction, Path::new("C:/vault/Дела.md"), &text).unwrap();
        transaction.commit().unwrap();
        let items = read_preview(&db, "C:/vault/Дела.md").unwrap();
        assert_eq!(items.len(), 65);
        assert_eq!(items[0].text, "task 0");
        assert_eq!(items[64].text, "task 64");
        assert!(read_preview(&db, "C:/other/Дела.md").unwrap().is_empty());
        let all = super::super::read(&db, &["C:/vault/Дела.md".to_owned()]).unwrap();
        assert_eq!(all["C:/vault/Дела.md"].len(), 80);
    }
}
