//! Read-only bridge contract for a selected native editor table.
//!
//! The host supplies the currently open editor model, including unsaved
//! changes. Byte offsets are ephemeral to that exact editor snapshot; they
//! are NEVER persistent source/table IDs or permission grants.
//! This code cannot read a vault, modify Markdown or invent exercise answers.

use std::ops::Range;

pub const MAX_TABLE_ROWS: usize = 4096;
pub const MAX_TABLE_COLUMNS: usize = 64;
pub const MAX_TABLE_CELL_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    NoActiveTable,
    InvalidDocumentRange,
    EmptyTable,
    TooManyRows,
    TooManyColumns,
    RaggedCells,
    OversizedCells,
    MergedCellsRequireReview,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditorTableSnapshot {
    /// Offset within one live editor revision, not a durable table identity.
    pub table_range: Range<usize>,
    /// Cells from the live parsed table, never a filesystem reread.
    pub cells: Vec<Vec<String>>,
}

/// Receive already parsed rows of the active editor table, not file bytes.
///
/// Merged cells are held until a semantic mapping can distinguish a shared
/// value from separate question/answer rows. Reject rather than invent cards.
pub fn capture(
    cells: &[Vec<String>],
    table_range: Range<usize>,
    document_len: usize,
    has_merges: bool,
) -> Result<EditorTableSnapshot, CaptureError> {
    if table_range.start >= table_range.end || table_range.end > document_len {
        return Err(CaptureError::InvalidDocumentRange);
    }
    if has_merges {
        return Err(CaptureError::MergedCellsRequireReview);
    }
    if cells.len() < 2 || cells.first().is_none_or(Vec::is_empty) {
        return Err(CaptureError::EmptyTable);
    }
    if cells.len() > MAX_TABLE_ROWS {
        return Err(CaptureError::TooManyRows);
    }
    let width = cells[0].len();
    if width > MAX_TABLE_COLUMNS {
        return Err(CaptureError::TooManyColumns);
    }
    let mut total_bytes = 0usize;
    for row in cells {
        if row.len() != width {
            return Err(CaptureError::RaggedCells);
        }
        for cell in row {
            total_bytes = total_bytes.saturating_add(cell.len());
            if total_bytes > MAX_TABLE_CELL_BYTES {
                return Err(CaptureError::OversizedCells);
            }
        }
    }
    Ok(EditorTableSnapshot { table_range, cells: cells.to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|row| row.iter().map(|cell| (*cell).to_owned()).collect()).collect()
    }

    #[test]
    fn active_table_is_copied_without_mutating_source_cells() {
        let original = table(&[&["English", "Русский"], &["ate", "съел"], &["last night", "вчера вечером"]]);
        let before = original.clone();
        let snapshot = capture(&original, 25..130, 500, false).unwrap();
        assert_eq!(snapshot.table_range, 25..130);
        assert_eq!(snapshot.cells[1], ["ate", "съел"]);
        assert_eq!(snapshot.cells[2], ["last night", "вчера вечером"]);
        assert_eq!(original, before);
    }

    #[test]
    fn body_ranges_must_be_valid_for_live_document() {
        let input = table(&[&["Q", "A"], &["eat", "есть"]]);
        assert_eq!(capture(&input, 20..20, 100, false), Err(CaptureError::InvalidDocumentRange));
        assert_eq!(capture(&input, 50..100, 99, false), Err(CaptureError::InvalidDocumentRange));
        assert_eq!(capture(&input, 100..10, 101, false), Err(CaptureError::InvalidDocumentRange));
    }

    #[test]
    fn merged_cells_and_ragged_rows_do_not_become_cards() {
        let input = table(&[&["Word", "RU"], &["run", "бегать"]]);
        assert_eq!(capture(&input, 0..20, 20, true), Err(CaptureError::MergedCellsRequireReview));
        let malformed = table(&[&["Word", "RU"], &["run"]]);
        assert_eq!(capture(&malformed, 0..20, 20, false), Err(CaptureError::RaggedCells));
    }

    #[test]
    fn physical_row_and_column_limits_apply_before_cloning() {
        let wide = vec![vec![String::new(); MAX_TABLE_COLUMNS + 1]; 2];
        assert_eq!(capture(&wide, 0..2, 2, false), Err(CaptureError::TooManyColumns));
        let tall = vec![vec!["X".to_owned()]; MAX_TABLE_ROWS + 1];
        assert_eq!(capture(&tall, 0..2, 2, false), Err(CaptureError::TooManyRows));
        assert_eq!(capture(&[], 0..2, 2, false), Err(CaptureError::EmptyTable));
    }

    #[test]
    fn large_text_is_refused_without_truncating_translations() {
        let large = vec![
            vec!["Question".to_owned(), "Answer".to_owned()],
            vec!["run".to_owned(), "я".repeat(MAX_TABLE_CELL_BYTES)],
        ];
        assert_eq!(capture(&large, 0..2, 2, false), Err(CaptureError::OversizedCells));
    }

    #[test]
    fn unicode_byte_ranges_are_opaque_and_values_stay_exact() {
        let original = table(&[&["Слово", "Перевод"], &["résumé", "резюме"]]);
        let out = capture(&original, 9..64, 70, false).unwrap();
        assert_eq!(out.table_range, 9..64);
        assert_eq!(out.cells[1][0], "résumé");
        assert_eq!(out.cells[1][1], "резюме");
    }
}
