//! Complete, bounded native editor snapshot for safe table identity binding.
//! The editor is windowed on large documents: never infer uniqueness from
//! only currently laid-out/rendered table widgets. Parse all live Markdown
//! (not disk bytes) with Akula's own table and Markdown block classifiers.

use super::*;
use crate::ui::editor::markdown::{Block, State};
use crate::ui::editor::table;
use aquilum_core::learning::editor_snapshot::EditorTableSnapshot;

const MAX_NOTE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TABLES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteScanError {
    NoActiveTable,
    TooLarge,
    TooManyTables,
    BadTable,
    ComplexTable,
    SelectionChanged,
}

pub struct StudyNoteSnapshot {
    pub selected: EditorTableSnapshot,
    pub selected_index: usize,
    /// Every table in the same trusted editor snapshot, including offscreen.
    pub tables: Vec<Vec<Vec<String>>>,
}

/// The selected range and cells must agree with the full live-document scan.
/// Fenced Markdown code and comments are skipped by the existing classifier.
fn collect(
    source: &str,
    selected: &EditorTableSnapshot,
) -> Result<(usize, Vec<Vec<Vec<String>>>), NoteScanError> {
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
    let mut selected_index = None;
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
            // The selected table's start is a TEMPORARY UI locator only.
            // The complete cell content must match the current table model.
            if begin == selected.table_range.start {
                if end != selected.table_range.end || parsed.cells != selected.cells {
                    return Err(NoteScanError::SelectionChanged);
                }
                selected_index = Some(tables.len());
            }
            tables.push(parsed.cells);
        }
    }
    let index = selected_index.ok_or(NoteScanError::SelectionChanged)?;
    Ok((index, tables))
}

impl TextEditor {
    /// Captures all Markdown tables from the open in-memory editor, even
    /// those outside the windowed layout. Never reads another note.
    pub fn study_complete_note(&self) -> Result<StudyNoteSnapshot, NoteScanError> {
        let selected = self.study_table_snapshot()
            .map_err(|_| NoteScanError::NoActiveTable)?;
        if self.doc.len() > MAX_NOTE_BYTES {
            return Err(NoteScanError::TooLarge);
        }
        let (selected_index, tables) = collect(&self.doc.whole(), &selected)?;
        Ok(StudyNoteSnapshot { selected, selected_index, tables })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::learning::editor_snapshot::capture;

    fn selected(source: &str, word: &str, answer: &str) -> EditorTableSnapshot {
        let start = source.find("| Q | A |").unwrap();
        let end = start + format!("| Q | A |\n| --- | --- |\n| {word} | {answer} |").len();
        let cells = vec![vec!["Q".into(), "A".into()], vec![word.into(), answer.into()]];
        capture(&cells, start..end, source.len(), false).unwrap()
    }

    #[test]
    fn all_visible_and_offscreen_tables_are_enumerated() {
        let source = "| Q | A |\n| --- | --- |\n| eat | есть |\n\n".to_owned()
            + "| Q | A |\n| --- | --- |\n| run | бегать |";
        let snap = selected(&source, "eat", "есть");
        let (index, tables) = collect(&source, &snap).unwrap();
        assert_eq!(index, 0);
        assert_eq!(tables.len(), 2);
        assert_eq!(tables[1][1], ["run", "бегать"]);
    }

    #[test]
    fn fenced_tables_do_not_create_false_anchor_matches() {
        let source = "| Q | A |\n| --- | --- |\n| eat | есть |\n\n"
            .to_owned() + "## Data\n\n" + "~~~md\n| Q | A |\n| --- | --- |\n| eat | есть |\n~~~";
        let snap = selected(&source, "eat", "есть");
        let (_, tables) = collect(&source, &snap).unwrap();
        assert_eq!(tables.len(), 1);
    }

    #[test]
    fn selected_table_content_must_still_match() {
        let source = "| Q | A |\n| --- | --- |\n| eat | есть |";
        let mut snap = selected(source, "eat", "есть");
        snap.cells[1][1] = "неверно".into();
        assert_eq!(collect(source, &snap).err(), Some(NoteScanError::SelectionChanged));
    }
    #[test]
    fn byte_budget_applies_before_table_parsing() {
        let source = "x".repeat(MAX_NOTE_BYTES + 1);
        let snap = capture(&vec![vec!["Q".into(), "A".into()], vec!["x".into(), "y".into()]],
            0..1, source.len(), false).unwrap();
        assert_eq!(collect(&source, &snap).err(), Some(NoteScanError::TooLarge));
    }
}
