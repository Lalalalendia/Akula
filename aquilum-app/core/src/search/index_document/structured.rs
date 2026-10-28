//! Only Markdown notes participate in wiki links, Dataview fields and tasks.
//! A plain TXT file is searchable text, not a Markdown document.
use super::PreparedFile;
use crate::search::created::{self, CreatedAt};
use crate::search::error::SearchError;
use crate::search::{fields, paths, tasks, wiki};
use rusqlite::Transaction;
use std::path::Path;

pub(super) fn created(file: &PreparedFile, fallback: CreatedAt) -> CreatedAt {
    if paths::is_markdown(&file.path) {
        created::declared(&file.body).unwrap_or(fallback)
    } else {
        fallback
    }
}

pub(super) fn index(
    root: &Path,
    transaction: &Transaction<'_>,
    file: &PreparedFile,
) -> Result<(), SearchError> {
    if paths::is_markdown(&file.path) {
        wiki::index_document(transaction, root, &file.path, &file.body)?;
        fields::index_document(transaction, &file.path, &file.body)?;
        tasks::index_document(transaction, &file.path, &file.body)?;
    }
    Ok(())
}
