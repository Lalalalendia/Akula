use super::SearchIndex;
use tantivy::query::{BooleanQuery, Occur, Query, TermQuery};
use tantivy::schema::{Field, IndexRecordOption};
use tantivy::Term;

pub(super) fn searchable_fields(index: &SearchIndex) -> [Field; 3] {
    [index.fields.title, index.fields.path, index.fields.body]
}

/// A leading '-' on a whitespace-delimited word excludes its indexed stem.
/// Ordinary multi-word queries keep their existing partial-match behavior.
pub(super) fn search_terms(input: &str) -> (Vec<String>, Vec<String>) {
    let mut positive = String::new();
    let mut excluded = Vec::new();
    for word in input.split_whitespace() {
        if let Some(word) = word.strip_prefix('-').filter(|word| !word.is_empty()) {
            excluded.extend(super::analyzed_query_terms(word));
        } else {
            if !positive.is_empty() {
                positive.push(' ');
            }
            positive.push_str(word);
        }
    }
    excluded.sort();
    excluded.dedup();
    // Preserve the legacy analyzer for queries without an exclusion.
    let positive = if excluded.is_empty() { input } else { &positive };
    (super::analyzed_query_terms(positive), excluded)
}

pub(super) fn add_exclusions(index: &SearchIndex, excluded: &[String], clauses: &mut Vec<(Occur, Box<dyn Query>)>) {
    for word in excluded {
        let fields: Vec<(Occur, Box<dyn Query>)> = searchable_fields(index)
            .into_iter()
            .map(|field| {
                let term = Term::from_field_text(field, word);
                (Occur::Should, Box::new(TermQuery::new(term, IndexRecordOption::Basic)) as Box<dyn Query>)
            })
            .collect();
        clauses.push((Occur::MustNot, Box::new(BooleanQuery::new(fields))));
    }
}
