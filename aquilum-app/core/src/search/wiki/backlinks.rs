use super::super::error::SearchError;
use super::super::models::Backlink;
use super::resolver::select_candidates;
use super::target::{relative_key, select_candidate, title_key};
use super::LINK_LIST_LIMIT;
use rusqlite::{params, Connection};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

const QUERY_BATCH_SIZE: usize = 256;

type LinkRow = (String, String, i64, i64);

pub fn backlinks(
    connection: &Connection,
    root: &Path,
    document: &Path,
) -> Result<Vec<Backlink>, SearchError> {
    let title = title_key(document);
    let relative = relative_key(root, document);
    let candidates = select_candidates(connection, &title)?;
    let mut statement = connection.prepare(
        "SELECT source_path, target_kind, byte_offset, offset_utf16 FROM wiki_links
         WHERE ((target_kind='name' AND target_key=?1)
            OR (target_kind='path' AND target_key=?2))
           AND (source_path > ?3 OR (source_path = ?3 AND byte_offset > ?4))
         ORDER BY source_path, byte_offset LIMIT ?5",
    )?;
    let mut cursor_path = String::new();
    let mut cursor_offset = -1;
    let mut items = Vec::new();
    while items.len() < LINK_LIST_LIMIT {
        let rows = query_batch(
            &mut statement,
            &title,
            &relative,
            &cursor_path,
            cursor_offset,
        )?;
        let Some(last) = rows.last() else { break };
        cursor_path = last.0.clone();
        cursor_offset = last.2;
        let exhausted = rows.len() < QUERY_BATCH_SIZE;
        for (source, target_kind, _byte_offset, offset_utf16) in rows {
            let source_path = PathBuf::from(&source);
            if target_kind == "name"
                && select_candidate(root, &source_path, &candidates).as_deref() != Some(document)
            {
                continue;
            }
            items.push(Backlink {
                path: source,
                title: source_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_owned(),
                offset: offset_utf16.max(0) as usize,
            });
            if items.len() == LINK_LIST_LIMIT {
                break;
            }
        }
        if exhausted {
            break;
        }
    }
    Ok(items)
}

/// Count distinct source notes without the display cap applied by `backlinks()`.
/// Use the same target-disambiguation logic as the list endpoint.
pub fn count_sources(
    connection: &Connection,
    root: &Path,
    document: &Path,
) -> Result<usize, SearchError> {
    let title = title_key(document);
    let relative = relative_key(root, document);
    let candidates = select_candidates(connection, &title)?;
    let mut statement = connection.prepare(
        "SELECT DISTINCT source_path, target_kind FROM wiki_links
         WHERE (target_kind='name' AND target_key=?1)
            OR (target_kind='path' AND target_key=?2)",
    )?;
    let mut sources = HashSet::new();
    let rows = statement.query_map(params![title, relative], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let own = document.to_string_lossy();
    for row in rows {
        let (source, kind) = row?;
        if source == own {
            continue;
        }
        if kind == "name"
            && select_candidate(root, Path::new(&source), &candidates).as_deref() != Some(document)
        {
            continue;
        }
        sources.insert(source);
    }
    Ok(sources.len())
}

fn query_batch(
    statement: &mut rusqlite::Statement<'_>,
    title: &str,
    relative: &str,
    cursor_path: &str,
    cursor_offset: i64,
) -> Result<Vec<LinkRow>, SearchError> {
    Ok(statement
        .query_map(
            params![
                title,
                relative,
                cursor_path,
                cursor_offset,
                QUERY_BATCH_SIZE as i64
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?
        .collect::<Result<Vec<_>, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::{backlinks, count_sources};
    use crate::search::wiki::repository::{index_document, open_schema};
    use rusqlite::Connection;
    use std::path::Path;

    #[test]
    fn counts_all_distinct_sources_beyond_the_ui_limit() {
        let mut connection = Connection::open_in_memory().unwrap();
        open_schema(&connection).unwrap();
        let root = Path::new("/vault");
        let target = root.join("Target.md");
        let tx = connection.transaction().unwrap();
        index_document(&tx, root, &target, "[[Target]]").unwrap(); // self-link excluded
        for n in 0..245 {
            let path = root.join(format!("Note-{n:03}.md"));
            let body = if n == 0 { "[[Target]] and [[Target]]" } else { "[[Target]]" };
            index_document(&tx, root, &path, body).unwrap();
        }
        index_document(&tx, root, &root.join("Unrelated.md"), "[[Other]]").unwrap();
        tx.commit().unwrap();

        assert_eq!(backlinks(&connection, root, &target).unwrap().len(), 200);
        assert_eq!(count_sources(&connection, root, &target).unwrap(), 245);
    }
}
