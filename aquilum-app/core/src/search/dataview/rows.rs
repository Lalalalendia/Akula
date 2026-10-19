use crate::search::error::SearchError;
use crate::search::fields::{read_fields, Field};
use crate::search::tasks::{read_tasks, Task};
use rusqlite::Connection;
use crate::search::paths::{relative_slash_path, strip_markdown_extension};
use std::path::Path;

pub struct Row {
    pub relative: String,
    pub source: String,
    pub name: String,
    pub folder: String,
    pub created: i64,
    pub modified: i64,
    pub size: i64,
    pub fields: Vec<Field>,
    pub tasks: Vec<Task>,
}

impl Row {
    pub fn field(&self, name: &str) -> Option<&Field> {
        let wanted = name.trim();
        if wanted.is_ascii() {
            // The common ASCII/ASCII case requires no heap allocations per
            // comparison. A Unicode key can still fold into ASCII (e.g. K -> k);
            // retain the original full Unicode lowercase behavior for it.
            let mut lowercase_ascii: Option<String> = None;
            self.fields.iter().find(|field| {
                let key = field.key.trim();
                if key.is_ascii() {
                    key.eq_ignore_ascii_case(wanted)
                } else {
                    key.to_lowercase()
                        == lowercase_ascii.get_or_insert_with(|| wanted.to_ascii_lowercase()).as_str()
                }
            })
        } else {
            let lowercase = wanted.to_lowercase();
            self.fields.iter().find(|field| {
                let key = field.key.trim();
                if key.is_ascii() {
                    key.eq_ignore_ascii_case(&lowercase)
                } else {
                    // str::to_lowercase is contextual for some Unicode strings.
                    // Per-char lowercase iterators are not a semantic replacement.
                    key.to_lowercase() == lowercase
                }
            })
        }
    }
}

use super::constants::CHUNK_PATHS as CHUNK;

pub fn load(
    connection: &Connection,
    root: &Path,
    paths: &[String],
) -> Result<Vec<Row>, SearchError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut fields = read_fields(connection, paths)?;
    let mut rows = Vec::with_capacity(paths.len());

    for chunk in paths.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT path, created_ns, modified_ns, size FROM documents
             WHERE path IN ({placeholders})"
        ))?;
        let found = statement.query_map(rusqlite::params_from_iter(chunk), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for entry in found {
            let (path, created, modified, size) = entry?;
            let relative = relative_slash_path(root, Path::new(&path));
            rows.push(Row {
                source: path.clone(),
                name: note_name(&relative),
                folder: folder_of(&relative),
                fields: fields.remove(&path).unwrap_or_default(),
                tasks: Vec::new(),
                relative,
                created,
                modified,
                size,
            });
        }
    }

    Ok(rows)
}

pub fn attach_tasks(connection: &Connection, rows: &mut [Row], paths: &[String]) -> Result<(), SearchError> {
    if paths.is_empty() || rows.is_empty() {
        return Ok(());
    }
    let mut found = read_tasks(connection, paths)?;
    for row in rows.iter_mut() {
        if let Some(tasks) = found.remove(&row.source) {
            row.tasks = tasks;
        }
    }
    Ok(())
}

fn note_name(relative: &str) -> String {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    strip_markdown_extension(name).to_owned()
}

fn folder_of(relative: &str) -> String {
    match relative.rfind('/') {
        Some(position) => relative[..position].to_owned(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{folder_of, note_name, Row};
    use crate::search::fields::Field;

    fn row(keys: &[&str]) -> Row {
        Row {
            relative: "folder/test.md".into(),
            source: "/tmp/test.md".into(),
            name: "test".into(),
            folder: "folder".into(),
            created: 0,
            modified: 0,
            size: 0,
            fields: keys.iter().map(|key| Field::scalar((*key).into(), "value".into())).collect(),
            tasks: Vec::new(),
        }
    }

    #[test]
    fn a_note_name_drops_the_folder_and_the_extension() {
        assert_eq!(note_name("Книги/Пороки.md"), "Пороки");
        assert_eq!(note_name("Пороки.md"), "Пороки");
    }

    #[test]
    fn a_note_in_the_root_has_an_empty_folder() {
        assert_eq!(folder_of("Книги/Пороки.md"), "Книги");
        assert_eq!(folder_of("Пороки.md"), "");
    }

    #[test]
    fn ascii_fields_keep_case_insensitive_first_match_priority() {
        let note = row(&["  Title  ", "TITLE", "other"]);
        assert_eq!(note.field("title").map(|f| f.key.as_str()), Some("  Title  "));
        assert_eq!(note.field(" TITLE ").map(|f| f.key.as_str()), Some("  Title  "));
        assert_eq!(note.field("OTHER").map(|f| f.key.as_str()), Some("other"));
        assert!(note.field("unlisted").is_none());
    }

    #[test]
    fn unicode_fields_match_previous_contextual_lowercase_semantics() {
        // Greek final sigma and non-ASCII characters which fold to ASCII
        // invalidate the simpler suggested chars().flat_map(to_lowercase) patch.
        let note = row(&["  ОСТАВИТЬ  ", "Key", "ΟΣ", "İ", "meta", "TITLE", "οσ", "école"]);
        let queries = [
            "оставить", " ОСТАВИТЬ ", "key", "KEY", "KEY", "ος", "ΟΣ",
            "οσ", "İ", "i\u{307}", "i", "META", "mEtA", "title",
            "ÉCOLE", "école", "nonexistent", "",
        ];
        for query in queries {
            let wanted = query.trim().to_lowercase();
            let previous = note.fields.iter()
                .find(|field| field.key.trim().to_lowercase() == wanted)
                .map(|field| field.key.as_str());
            let current = note.field(query).map(|field| field.key.as_str());
            assert_eq!(current, previous, "field lookup changed for {query:?}");
        }
    }

    #[test]
    fn unicode_ascii_pairs_match_old_algorithm_in_both_directions() {
        let samples = ["A", "a", "K", "k", "K", "Σ", "σ", "ς", "ΟΣ", "ος", "οσ", "İ", "i\u{307}", "ТЕСТ", "тест", "ẞ", "ß", "TITLE", "title", " x "];
        let note = row(&samples);
        for name in samples.iter().chain(["missing", "Ω", "É", "é", "STRASSE"].iter()) {
            let wanted = name.trim().to_lowercase();
            let original = note.fields.iter().find(|field| field.key.trim().to_lowercase() == wanted);
            let current = note.field(name);
            assert_eq!(current.map(|f| f.key.as_str()), original.map(|f| f.key.as_str()), "name={name}");
        }
    }
}
