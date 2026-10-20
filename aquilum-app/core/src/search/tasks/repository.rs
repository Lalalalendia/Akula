use super::parse::{note_tasks, Task};
use crate::search::error::SearchError;
use rusqlite::{params, Connection, Transaction};
use std::collections::HashMap;
use std::path::Path;

pub fn open_schema(connection: &Connection) -> Result<(), SearchError> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS note_tasks (
           path TEXT NOT NULL,
           ordinal INTEGER NOT NULL,
           line INTEGER NOT NULL,
           done INTEGER NOT NULL,
           text TEXT NOT NULL,
           PRIMARY KEY(path, ordinal)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS note_tasks_open ON note_tasks(done);",
    )?;
    Ok(())
}

pub fn index_document(
    transaction: &Transaction<'_>,
    path: &Path,
    body: &str,
) -> Result<(), SearchError> {
    remove_document(transaction, path)?;
    #[cfg(feature = "index-stage-probe")]
    let found = crate::search::perf_probe::time(crate::search::perf_probe::Stage::TasksParse, || note_tasks(body));
    #[cfg(not(feature = "index-stage-probe"))]
    let found = note_tasks(body);
    #[cfg(feature = "index-stage-probe")]
    crate::search::perf_probe::record_rows(crate::search::perf_probe::Stage::Tasks, found.len());
    if found.is_empty() {
        return Ok(());
    }
    let path_text = path.to_string_lossy();
    let mut statement = transaction.prepare(
        "INSERT INTO note_tasks(path, ordinal, line, done, text) VALUES(?1, ?2, ?3, ?4, ?5)",
    )?;
    for (ordinal, task) in found.iter().enumerate() {
        statement.execute(params![
            path_text,
            ordinal as i64,
            task.line as i64,
            i64::from(task.done),
            task.text,
        ])?;
    }
    Ok(())
}

pub fn remove_document(transaction: &Transaction<'_>, path: &Path) -> Result<(), SearchError> {
    transaction.execute(
        "DELETE FROM note_tasks WHERE path=?1",
        [path.to_string_lossy().as_ref()],
    )?;
    Ok(())
}

const CHUNK: usize = 400;

pub fn read(
    connection: &Connection,
    paths: &[String],
) -> Result<HashMap<String, Vec<Task>>, SearchError> {
    let mut found: HashMap<String, Vec<Task>> = HashMap::new();
    for chunk in paths.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT path, line, done, text FROM note_tasks
             WHERE path IN ({placeholders}) ORDER BY path, ordinal"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(chunk), |row| {
            Ok((
                row.get::<_, String>(0)?,
                Task {
                    line: row.get::<_, i64>(1)? as usize,
                    done: row.get::<_, i64>(2)? != 0,
                    text: row.get::<_, String>(3)?,
                },
            ))
        })?;
        for entry in rows {
            let (path, task) = entry?;
            found.entry(path).or_default().push(task);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{index_document, open_schema, read, remove_document};
    use rusqlite::Connection;
    use std::path::Path;

    fn store(body: &str) -> Connection {
        let mut connection = Connection::open_in_memory().expect("память");
        open_schema(&connection).expect("схема");
        let transaction = connection.transaction().expect("транзакция");
        index_document(&transaction, Path::new("C:/vault/Дела.md"), body).expect("индексация");
        transaction.commit().expect("фиксация");
        connection
    }

    #[test]
    fn tasks_survive_a_round_trip_through_the_index() {
        let connection = store("- [ ] написать\n- [x] отправить");
        let found = read(&connection, &["C:/vault/Дела.md".to_owned()]).expect("чтение");
        let tasks = &found["C:/vault/Дела.md"];
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].text, "написать");
        assert!(!tasks[0].done);
        assert!(tasks[1].done);
    }

    #[test]
    fn reindexing_replaces_the_previous_tasks() {
        let mut connection = store("- [ ] старая");
        let transaction = connection.transaction().expect("транзакция");
        index_document(&transaction, Path::new("C:/vault/Дела.md"), "- [ ] новая").expect("снова");
        transaction.commit().expect("фиксация");

        let found = read(&connection, &["C:/vault/Дела.md".to_owned()]).expect("чтение");
        assert_eq!(found["C:/vault/Дела.md"].len(), 1);
        assert_eq!(found["C:/vault/Дела.md"][0].text, "новая");
    }

    #[test]
    fn removing_a_document_takes_its_tasks_with_it() {
        let mut connection = store("- [ ] написать");
        let transaction = connection.transaction().expect("транзакция");
        remove_document(&transaction, Path::new("C:/vault/Дела.md")).expect("удаление");
        transaction.commit().expect("фиксация");

        let found = read(&connection, &["C:/vault/Дела.md".to_owned()]).expect("чтение");
        assert!(found.is_empty());
    }
}
