use super::negative::searchable_fields;
use crate::search::analyzer::indexing_tokenizer;
use crate::search::error::SearchError;
use crate::search::index::SearchIndex;
use std::path::Path;
use tantivy::query::{BooleanQuery, Occur, PhraseQuery, Query, TermQuery, TermSetQuery};
use tantivy::schema::{Field, IndexRecordOption};
use tantivy::tokenizer::TokenStream;
use tantivy::{Searcher, Term};

/// Extract complete double-quoted sequences in the same order as the index.
/// The analyzer may remove stop words, so this is an indexed word sequence,
/// not a byte-for-byte comparison of punctuation or spelling.
pub(super) fn quoted_phrases(input: &str) -> Vec<(String, Vec<String>)> {
    let mut phrases = Vec::new();
    let mut rest = input;
    while let Some(begin) = rest.find('"') {
        let quoted = &rest[begin + 1..];
        let Some(end) = quoted.find('"') else {
            break;
        };
        if !rest[..begin].ends_with('-') {
            let mut words = Vec::new();
            indexing_tokenizer().token_stream(&quoted[..end]).process(&mut |token| {
                if !token.text.is_empty() {
                    words.push(token.text.clone());
                }
            });
            if !words.is_empty() {
                phrases.push((quoted[..end].to_owned(), words));
            }
        }
        rest = &quoted[end + 1..];
    }
    phrases
}

pub(super) fn folder_query(searcher: &Searcher, id: Field, folder: &Path) -> Result<TermSetQuery, SearchError> {
    let mut inside = Vec::new();
    for segment in searcher.segment_readers() {
        let inverted = segment.inverted_index(id)?;
        let mut paths = inverted.terms().stream()?;
        while paths.advance() {
            if let Ok(path) = std::str::from_utf8(paths.key()) {
                if Path::new(path).starts_with(folder) {
                    inside.push(Term::from_field_text(id, path));
                }
            }
        }
    }
    Ok(TermSetQuery::new(inside))
}

pub(super) fn extra_clauses(
    index: &SearchIndex,
    phrases: &[(String, Vec<String>)],
) -> Vec<(Occur, Box<dyn Query>)> {
    let mut clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    // Apply phrase constraints within Tantivy, before taking the top N results,
    // so filtering cannot silently drop valid matches beyond the first page.
    for (_, words) in phrases {
        let fields: Vec<(Occur, Box<dyn Query>)> = searchable_fields(index)
            .into_iter()
            .map(|field| {
                let terms: Vec<Term> = words
                    .iter()
                    .map(|word| Term::from_field_text(field, word))
                    .collect();
                let matcher: Box<dyn Query> = if terms.len() == 1 {
                    Box::new(TermQuery::new(terms[0].clone(), IndexRecordOption::Basic))
                } else {
                    Box::new(PhraseQuery::new(terms))
                };
                (Occur::Should, matcher)
            })
            .collect();
        clauses.push((Occur::Must, Box::new(BooleanQuery::new(fields))));
    }
    clauses
}
