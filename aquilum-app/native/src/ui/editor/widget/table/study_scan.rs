//! Shared bounded Akula Markdown table scan for active editor and trusted
//! first-party LANG source reads. Never parse only visible editor widgets.
//!
//! The caller is responsible for file read authorization and fresh revision;
//! scanning plain text is not a grant to open a vault.

use std::ops::Range;
use super::study_note::NoteScanError;
use super::TextEditor;
use crate::ui::editor::markdown::{Block, State};
use crate::ui::editor::table;

const MAX_NOTE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TABLES: usize = 256;

pub(crate) struct ScannedTable {
    pub range: Range<usize>,
    pub cells: Vec<Vec<String>>,
}

pub(crate) fn scan_tables(source: &str) -> Result<Vec<ScannedTable>, NoteScanError> {
    if source.len() > MAX_NOTE_BYTES {
        return Err(NoteScanError::TooLarge);
    }
    let lines: Vec<&str> = source.split('\n').collect();
    let mut offsets = Vec::with_capacity(lines.len());
    let mut at = 0usize;
    for line in &lines {
        offsets.push(at);
        at = at.saturating_add(line.len() + 1);
    }

    let mut state = State::default();
    let mut first = None;
    let mut tables = Vec::new();
    for (line_index, line) in lines.iter().enumerate() {
        let next = lines.get(line_index + 1).copied();
        let block = state.step(line, next);
        if matches!(block, Block::Table { first: true, .. }) {
            first = Some(line_index);
        }
        if matches!(block, Block::Table { last: true, .. }) {
            let start = first.take().ok_or(NoteScanError::BadTable)?;
            let parsed = table::parse_with(&lines[start..=line_index], None)
                .ok_or(NoteScanError::BadTable)?;
            if !parsed.merges.is_empty() {
                return Err(NoteScanError::ComplexTable);
            }
            if tables.len() >= MAX_TABLES {
                return Err(NoteScanError::TooManyTables);
            }
            let begin = offsets[start];
            let end = offsets[line_index] + lines[line_index].len();
            tables.push(ScannedTable {
                range: begin..end,
                cells: parsed.cells,
            });
        }
    }
    Ok(tables)
}

impl TextEditor {
    /// Scan a string already authorized and read by the trusted native host.
    /// Unlike study_complete_note this does not require a selected UI table.
    /// No content is cached to the study metadata database.
    pub(crate) fn scan_study_source(
        source: &str,
    ) -> Result<Vec<Vec<Vec<String>>>, NoteScanError> {
        scan_tables(source).map(|tables| tables.into_iter().map(|t| t.cells).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_classifier_ignores_fenced_and_comment_tables() {
        let body = "| Q | A |\n| --- | --- |\n| eat | есть |\n\n"
            .to_owned()
            + "~~~md\n| Q | A |\n| --- | --- |\n| fake | неверно |\n~~~\n\n"
            + "| Q | A |\n| --- | --- |\n| read | читать |";
        let parsed = TextEditor::scan_study_source(&body).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0][1], ["eat", "есть"]);
        assert_eq!(parsed[1][1], ["read", "читать"]);
    }

    #[test]
    fn bounded_scan_refuses_oversized_markdown_before_parsing() {
        assert_eq!(
            TextEditor::scan_study_source(&"x".repeat(MAX_NOTE_BYTES + 1)).err(),
            Some(NoteScanError::TooLarge)
        );
    }

    #[test]
    fn scanning_two_tables_keeps_complete_ranges_in_source_order() {
        let text = "| Q | A |\n| --- | --- |\n| one | один |\n\n"
            .to_owned() + "| Q | A |\n| --- | --- |\n| two | два |";
        let parsed = scan_tables(&text).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(&text[parsed[0].range.clone()], "| Q | A |\n| --- | --- |\n| one | один |");
        assert_eq!(&text[parsed[1].range.clone()], "| Q | A |\n| --- | --- |\n| two | два |");
    }
}
