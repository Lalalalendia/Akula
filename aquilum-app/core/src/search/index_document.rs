mod structured;
use super::paths::relative_slash_path;
use super::created::{self, CreatedAt, CreatedSource};
use super::error::SearchError;
use super::fields;
use super::tasks;
use super::index::SearchIndex;
use super::metadata::{self, FileState, Fingerprint, PathKey};
use super::wiki;
use super::ANALYZER_VERSION;
use rusqlite::{params, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tantivy::{doc, IndexWriter, Term};
use tantivy::collector::Count;
use tantivy::query::TermQuery;
use tantivy::schema::IndexRecordOption;

pub struct PreparedFile {
    pub path: PathBuf,
    pub fingerprint: Fingerprint,
    pub content_hash: [u8; 32],
    pub body: String,
}

#[derive(Clone, Copy)]
pub struct Probe {
    pub fingerprint: Fingerprint,
    pub up_to_date: bool,
    pub previous_hash: Option<[u8; 32]>,
    pub created: CreatedAt,
}

pub fn probe(
    path: &Path,
    known: &mut HashMap<PathKey, FileState>,
    force: bool,
) -> Option<(String, Probe)> {
    let key = path.to_string_lossy().into_owned();
    let Ok(file_metadata) = path.metadata() else {
        return None;
    };
    let previous = known.remove(&metadata::key(&key));
    classify(
        previous,
        metadata::fingerprint(&file_metadata),
        created::from_file_metadata(&file_metadata),
        force,
    )
    .map(|probe| (key, probe))
}

pub fn prepare_file(path: PathBuf, fingerprint: Fingerprint) -> Option<PreparedFile> {
    let body = crate::files::document::read_text(&path).ok()?;
    let content_hash = *blake3::hash(body.as_bytes()).as_bytes();
    Some(PreparedFile {
        path,
        fingerprint,
        content_hash,
        body,
    })
}

pub fn apply_prepared(
    root: &Path,
    index: &SearchIndex,
    writer: &IndexWriter,
    transaction: &Transaction<'_>,
    key: &str,
    probe: Probe,
    prepared: PreparedFile,
) -> Result<bool, SearchError> {
    let created = structured::created(&prepared, probe.created);
    if probe.up_to_date
        && probe
            .previous_hash
            .is_some_and(|hash| hash == prepared.content_hash)
    {
        upsert(transaction, key, &prepared, created)?;
        return Ok(false);
    }
    update_document(root, index, writer, &prepared.path, &prepared.body)?;
    structured::index(root, transaction, &prepared)?;
    upsert(transaction, key, &prepared, created)?;
    Ok(true)
}

/// Ignore a deletion notification only when neither metadata nor the
/// committed search index contains that path. Checking both keeps recovery
/// from a partially committed index/SQLite update possible.
pub fn untracked_missing_path(
    index: &SearchIndex,
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<bool, SearchError> {
    if path.is_file() {
        return Ok(false);
    }
    let key = path.to_string_lossy();
    if metadata::current(transaction, &key)?.is_some() {
        return Ok(false);
    }
    let query = TermQuery::new(
        Term::from_field_text(index.fields.id, &key),
        IndexRecordOption::Basic,
    );
    let searcher = index.reader.searcher();
    Ok(searcher.search(&query, &Count)? == 0)
}

pub fn apply_path(
    root: &Path,
    index: &SearchIndex,
    writer: &IndexWriter,
    transaction: &Transaction<'_>,
    path: &Path,
) -> Result<bool, SearchError> {
    let key = path.to_string_lossy();
    if !path.is_file() {
        writer.delete_term(Term::from_field_text(index.fields.id, &key));
        wiki::remove_document(transaction, path)?;
        fields::remove_document(transaction, path)?;
        tasks::remove_document(transaction, path)?;
        transaction.execute("DELETE FROM documents WHERE path=?1", params![key])?;
        return Ok(true);
    }

    let file_metadata = path.metadata()?;
    let fingerprint = metadata::fingerprint(&file_metadata);
    let Some(probe) = classify(
        metadata::current(transaction, &key)?,
        fingerprint,
        created::from_file_metadata(&file_metadata),
        false,
    ) else {
        return Ok(false);
    };
    let Some(prepared) = prepare_file(path.to_path_buf(), fingerprint) else {
        return Ok(false);
    };
    apply_prepared(root, index, writer, transaction, &key, probe, prepared)
}

fn classify(
    previous: Option<FileState>,
    fingerprint: Fingerprint,
    candidate: CreatedAt,
    force: bool,
) -> Option<Probe> {
    let up_to_date = previous
        .as_ref()
        .is_some_and(|value| value.analyzer_version == ANALYZER_VERSION);
    if !force && up_to_date && previous.as_ref().is_some_and(|value| value.fingerprint == fingerprint)
    {
        return None;
    }
    let carried = previous
        .as_ref()
        .map(|value| value.created)
        .filter(|value| value.source != CreatedSource::Frontmatter);
    let created = created::resolve(carried, candidate);
    Some(Probe {
        fingerprint,
        up_to_date: !force && up_to_date,
        previous_hash: previous.and_then(|value| value.content_hash.try_into().ok()),
        created,
    })
}

fn update_document(
    root: &Path,
    index: &SearchIndex,
    writer: &IndexWriter,
    path: &Path,
    body: &str,
) -> Result<(), SearchError> {
    let absolute_path = path.to_string_lossy();
    let searchable_path = relative_slash_path(root, path);
    let title = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    writer.delete_term(Term::from_field_text(index.fields.id, &absolute_path));
    writer.add_document(doc!(
        index.fields.id => absolute_path.as_ref(),
        index.fields.path => searchable_path,
        index.fields.title => title,
        index.fields.body => body,
    ))?;
    Ok(())
}

fn upsert(
    transaction: &Transaction<'_>,
    path: &str,
    prepared: &PreparedFile,
    created: CreatedAt,
) -> Result<(), SearchError> {
    transaction.execute(
        "INSERT INTO documents(path, modified_ns, size, content_hash, analyzer_version,
           created_ns, created_source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(path) DO UPDATE SET modified_ns=excluded.modified_ns,
           size=excluded.size, content_hash=excluded.content_hash,
           analyzer_version=excluded.analyzer_version,
           created_ns=excluded.created_ns, created_source=excluded.created_source",
        params![
            path,
            prepared.fingerprint.0,
            prepared.fingerprint.1,
            &prepared.content_hash[..],
            ANALYZER_VERSION,
            created.nanos,
            created.source.stored(),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{classify, CreatedAt, CreatedSource};
    use crate::search::metadata::FileState;

    fn state(created: CreatedAt) -> FileState {
        FileState {
            fingerprint: (1, 1),
            content_hash: vec![0u8; 32],
            analyzer_version: 0,
            created,
        }
    }

    fn from_file() -> CreatedAt {
        CreatedAt {
            nanos: 500,
            source: CreatedSource::FileBirthTime,
        }
    }

    fn declared_at(nanos: i64) -> CreatedAt {
        CreatedAt {
            nanos,
            source: CreatedSource::Frontmatter,
        }
    }

    #[test]
    fn a_removed_declaration_hands_the_date_back_to_the_file() {
        let probe = classify(Some(state(declared_at(900))), (2, 2), from_file(), false)
            .expect("changed file is probed");

        assert_eq!(probe.created, from_file());
    }

    #[test]
    fn a_crlf_note_is_indexed_as_lf() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("Заметка.md");
        std::fs::write(&path, "строка
вторая
").unwrap();
        let prepared = super::prepare_file(path, (1, 1)).unwrap();
        assert_eq!(prepared.body, "строка
вторая
");
    }
}
