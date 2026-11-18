//! LANG-01: read-only study preview from an existing table's cells.
//!
//! This does not parse Markdown, read or write files, persist identities,
//! or authorize a vault. The trusted caller must obtain current table cells
//! from an authorized editor snapshot. A row index here is an ephemeral
//! snapshot locator, NEVER a durable key for review history.

pub mod identity;
pub mod collections;
pub mod store;
pub mod editor_snapshot;
use std::collections::BTreeSet;

/// Explicit column roles chosen for each source table.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnMapping {
    pub question: usize,
    /// Columns containing independent accepted answer variants.
    /// Comma-separated text is deliberately NOT split automatically.
    pub answers: Vec<usize>,
    pub context: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MappingError {
    MissingHeader,
    MissingAnswers,
    ColumnOutOfRange { index: usize, width: usize },
    ColumnUsedTwice { index: usize },
}

impl ColumnMapping {
    pub fn validate(&self, width: usize) -> Result<(), MappingError> {
        if width == 0 {
            return Err(MappingError::MissingHeader);
        }
        if self.answers.is_empty() {
            return Err(MappingError::MissingAnswers);
        }
        let mut seen = BTreeSet::new();
        for index in std::iter::once(self.question)
            .chain(self.answers.iter().copied())
            .chain(self.context)
        {
            if index >= width {
                return Err(MappingError::ColumnOutOfRange { index, width });
            }
            if !seen.insert(index) {
                return Err(MappingError::ColumnUsedTwice { index });
            }
        }
        Ok(())
    }
}

/// One question from the current table snapshot. source_row is not a stable ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudyItem {
    pub source_row: usize,
    pub question: String,
    pub accepted_answers: Vec<String>,
    pub context: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowProblem {
    WrongWidth,
    EmptyQuestion,
    EmptyAnswer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedRow {
    pub source_row: usize,
    pub problem: RowProblem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TablePreview {
    pub items: Vec<StudyItem>,
    pub skipped: Vec<SkippedRow>,
    pub truncated: bool,
}

/// Preview only already-parsed, authorized cell values; row 0 is the header.
/// max_rows limits PHYSICAL data rows, including empty or invalid entries.
/// The function never modifies the table or stores unverified source IDs.
pub fn preview_table(
    rows: &[Vec<String>],
    mapping: &ColumnMapping,
    max_rows: usize,
) -> Result<TablePreview, MappingError> {
    let header = rows.first().ok_or(MappingError::MissingHeader)?;
    let width = header.len();
    mapping.validate(width)?;
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    let data_rows = rows.len().saturating_sub(1);
    for (source_row, cells) in rows.iter().enumerate().skip(1).take(max_rows) {
        if cells.len() != width {
            skipped.push(SkippedRow { source_row, problem: RowProblem::WrongWidth });
            continue;
        }
        let question = cells[mapping.question].trim();
        if question.is_empty() {
            skipped.push(SkippedRow { source_row, problem: RowProblem::EmptyQuestion });
            continue;
        }
        // Do not conflate distinct meanings or infer synonyms.
        let mut accepted_answers: Vec<String> = Vec::new();
        for column in &mapping.answers {
            let value = cells[*column].trim();
            if !value.is_empty() && !accepted_answers.iter().any(|seen| seen.as_str() == value) {
                accepted_answers.push(value.to_owned());
            }
        }
        if accepted_answers.is_empty() {
            skipped.push(SkippedRow { source_row, problem: RowProblem::EmptyAnswer });
            continue;
        }
        let context = mapping.context.and_then(|column| {
            let value = cells[column].trim();
            (!value.is_empty()).then(|| value.to_owned())
        });
        items.push(StudyItem {
            source_row,
            question: question.to_owned(),
            accepted_answers,
            context,
        });
    }
    Ok(TablePreview { items, skipped, truncated: data_rows > max_rows })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn table(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|row| row.iter().map(|cell| (*cell).to_owned()).collect()).collect()
    }

    #[test]
    fn explicit_roles_support_alternative_translations_without_splitting() {
        let rows = table(&[
            &["Word", "Meaning 1", "Meaning 2", "Context"],
            &["fast", "быстрый", "скорый", "movement"],
            &["last night", "вчера вечером", "прошлой ночью", "time"],
        ]);
        let original = rows.clone();
        let mapping = ColumnMapping { question: 0, answers: vec![1, 2], context: Some(3) };
        let preview = preview_table(&rows, &mapping, 10).unwrap();
        assert_eq!(preview.items.len(), 2);
        assert_eq!(preview.items[0].accepted_answers[0], "быстрый");
        assert_eq!(preview.items[0].accepted_answers[1], "скорый");
        assert_eq!(preview.items[0].context.as_deref(), Some("movement"));
        assert_eq!(preview.items[1].source_row, 2);
        assert!(preview.skipped.is_empty());
        assert!(!preview.truncated);
        assert_eq!(rows, original, "preview must never modify the source");
    }
    #[test]
    fn repeated_spelling_with_distinct_contexts_remains_separate() {
        let rows = table(&[
            &["EN", "RU", "Example"],
            &["run", "бегать", "run every day"],
            &["run", "работать", "the program runs"],
            &["run", "управлять", "run a company"],
        ]);
        let mapping = ColumnMapping { question: 0, answers: vec![1], context: Some(2) };
        let preview = preview_table(&rows, &mapping, 10).unwrap();
        assert_eq!(preview.items.len(), 3);
        assert_eq!(preview.items[2].source_row, 3);
        assert_eq!(preview.items[2].context.as_deref(), Some("run a company"));
    }
    #[test]
    fn arbitrary_term_definition_table_uses_explicit_columns() {
        let rows = table(&[&["Определение", "Термин"], &["Синтез белка", "Рибосома"]]);
        let mapping = ColumnMapping { question: 1, answers: vec![0], context: None };
        let preview = preview_table(&rows, &mapping, 5).unwrap();
        assert_eq!(preview.items[0].question, "Рибосома");
        assert_eq!(preview.items[0].accepted_answers[0], "Синтез белка");
    }

    #[test]
    fn invalid_rows_are_reported_not_made_into_questions() {
        let rows = table(&[
            &["EN", "RU"],
            &["courier", "курьер"],
            &["empty", ""],
            &["", "ночь"],
            &["broken"],
        ]);
        let mapping = ColumnMapping { question: 0, answers: vec![1], context: None };
        let preview = preview_table(&rows, &mapping, 10).unwrap();
        assert_eq!(preview.items.len(), 1);
        assert_eq!(
            preview.skipped,
            [
                SkippedRow { source_row: 2, problem: RowProblem::EmptyAnswer },
                SkippedRow { source_row: 3, problem: RowProblem::EmptyQuestion },
                SkippedRow { source_row: 4, problem: RowProblem::WrongWidth },
            ]
        );
    }

    #[test]
    fn scan_limit_bounds_physical_rows_not_only_usable_entries() {
        let rows = table(&[&["EN", "RU"], &["", ""], &["eat", "есть"], &["ate", "съел"]]);
        let mapping = ColumnMapping { question: 0, answers: vec![1], context: None };
        let preview = preview_table(&rows, &mapping, 2).unwrap();
        assert_eq!(preview.items.len(), 1);
        assert_eq!(preview.items[0].source_row, 2);
        assert!(preview.truncated);
        let zero = preview_table(&rows, &mapping, 0).unwrap();
        assert!(zero.items.is_empty());
        assert!(zero.truncated);
    }

    #[test]
    fn invalid_roles_fail_closed() {
        let rows = table(&[&["A", "B"], &["x", "y"]]);
        assert_eq!(
            preview_table(&rows, &ColumnMapping { question: 0, answers: vec![], context: None }, 5),
            Err(MappingError::MissingAnswers)
        );
        assert_eq!(
            preview_table(&rows, &ColumnMapping { question: 0, answers: vec![0], context: None }, 5),
            Err(MappingError::ColumnUsedTwice { index: 0 })
        );
        assert_eq!(
            preview_table(&rows, &ColumnMapping { question: 0, answers: vec![2], context: None }, 5),
            Err(MappingError::ColumnOutOfRange { index: 2, width: 2 })
        );
        assert_eq!(
            preview_table(&[], &ColumnMapping { question: 0, answers: vec![1], context: None }, 5),
            Err(MappingError::MissingHeader)
        );
    }

    #[test]
    fn identical_answer_cells_yield_one_accepted_variant() {
        let rows = table(&[&["EN", "RU1", "RU2"], &["eat", " есть ", "есть"]]);
        let mapping = ColumnMapping { question: 0, answers: vec![1, 2], context: None };
        let preview = preview_table(&rows, &mapping, 5).unwrap();
        assert_eq!(preview.items[0].accepted_answers, ["есть".to_owned()]);
    }
}
