use super::value::{key_lower, value_lower, Field, FieldKind};
use crate::search::error::SearchError;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

const CHUNK: usize = 400;

pub fn read(
    connection: &Connection,
    paths: &[String],
) -> Result<HashMap<String, Vec<Field>>, SearchError> {
    let mut found: HashMap<String, Vec<Field>> = HashMap::new();
    for chunk in paths.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let mut statement = connection.prepare(&format!(
            "SELECT path, key, kind, text, items FROM note_fields
             WHERE path IN ({placeholders}) ORDER BY path, ordinal"
        ))?;
        let rows = statement.query_map(rusqlite::params_from_iter(chunk), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
            ))
        })?;
        for row in rows {
            let (path, key, kind, text, items) = row?;
            found.entry(path).or_default().push(Field::stored(
                key,
                FieldKind::from_stored(kind),
                text,
                items,
            ));
        }
    }
    Ok(found)
}

pub fn candidates(
    connection: &Connection,
    equals: &[(String, String)],
    has: &[String],
) -> Result<Vec<String>, SearchError> {
    let mut narrowed: Option<HashSet<String>> = None;
    for (key, value) in equals {
        let found = lookup(
            connection,
            "SELECT DISTINCT path FROM note_fields WHERE key_lower=?1 AND text_lower=?2",
            &[key_lower(key), value_lower(value)],
        )?;
        narrowed = Some(intersect(narrowed, found));
    }
    for key in has {
        let found = lookup(
            connection,
            "SELECT DISTINCT path FROM note_fields WHERE key_lower=?1 AND text_lower<>''",
            &[key_lower(key)],
        )?;
        narrowed = Some(intersect(narrowed, found));
    }
    match narrowed {
        Some(paths) => {
            let mut paths = paths.into_iter().collect::<Vec<_>>();
            paths.sort();
            Ok(paths)
        }
        None => all_paths(connection),
    }
}

pub fn paths_with_tag(connection: &Connection, tag: &str) -> Result<HashSet<String>, SearchError> {
    let wanted = value_lower(tag.trim_start_matches('#'));
    let nested = format!("{wanted}/");
    let mut statement =
        connection.prepare("SELECT path, text_lower, items FROM note_fields WHERE key_lower='tags'")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut found = HashSet::new();
    for row in rows {
        let (path, text, items) = row?;
        let carries = |value: &str| {
            let value = value_lower(value);
            value == wanted || value.starts_with(&nested)
        };
        let matched = match items {
            Some(raw) => serde_json::from_str::<Vec<String>>(&raw)
                .map(|items| items.iter().any(|item| carries(item)))
                .unwrap_or(false),
            None => carries(&text),
        };
        if matched {
            found.insert(path);
        }
    }
    Ok(found)
}

fn lookup(
    connection: &Connection,
    sql: &str,
    arguments: &[String],
) -> Result<HashSet<String>, SearchError> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(arguments), |row| {
            row.get::<_, String>(0)
        })?
        .collect::<Result<HashSet<_>, _>>()?;
    Ok(rows)
}

pub(in crate::search) fn all_paths(connection: &Connection) -> Result<Vec<String>, SearchError> {
    let mut statement = connection.prepare("SELECT path FROM documents ORDER BY path")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn intersect(narrowed: Option<HashSet<String>>, found: HashSet<String>) -> HashSet<String> {
    match narrowed {
        Some(current) => current.intersection(&found).cloned().collect(),
        None => found,
    }
}
