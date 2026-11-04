mod count;
mod excerpt;
mod file;
mod window;

use super::error::SearchError;
use super::headings::heading_at;
use super::index::{stored_text, SearchFields};
use super::models::SearchResult;
use crate::text::cancel::Cancel;
use crate::text::matcher::{Fold, Matcher, Mode, Options, EXACT};
use excerpt::{excerpt, CHARS_AFTER_MATCH};
use file::MatchWindow;
use std::path::Path;
use tantivy::{DocAddress, Searcher, TantivyDocument};

pub const QUERY_MATCH: Options = Options { fold: Fold::Ascii, mode: Mode::WordPrefix, ..EXACT };

pub fn query_matcher<S: AsRef<str>>(terms: &[S]) -> Matcher {
    Matcher::any(terms, QUERY_MATCH)
}

pub fn results(
    searcher: &Searcher,
    fields: SearchFields,
    terms: &[String],
    quoted: &[String],
    addresses: &[DocAddress],
    cancel: &Cancel,
) -> Result<Vec<SearchResult>, SearchError> {
    let counts = count::body_counts(searcher, fields.body, terms, addresses)?;
    let matcher = query_matcher(terms);
    // Quoted phrases are a stricter search than isolated terms. Prefer an
    // actual literal sequence for the jump position and result excerpt.
    // Indexed stemming may match another spelling; keep the term fallback.
    let quoted_matcher = (!quoted.is_empty()).then(|| {
        Matcher::any(quoted, Options { mode: Mode::WholeWord, ..QUERY_MATCH })
    });
    // These are query-scoped, not file-scoped: avoid allocating a matcher for
    // each term of every search result when checking the filename.
    let term_matchers: Vec<_> = terms
        .iter()
        .map(|term| query_matcher(std::slice::from_ref(term)))
        .collect();
    let mut results = Vec::with_capacity(addresses.len());
    for (address, body) in addresses.iter().zip(counts) {
        if cancel.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
        let document = searcher.doc::<TantivyDocument>(*address)?;
        let Some(path) = stored_text(&document, fields.id) else {
            continue;
        };
        let title = stored_text(&document, fields.title).unwrap_or_default();
        let file = Path::new(&path);
        let has_body_match = body.iter().any(|&count| count > 0);
        let window = window::read_window(
            file, has_body_match, &matcher, quoted_matcher.as_ref(), cancel,
        );
        if cancel.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
        results.push(result(path, title, terms, &matcher, &term_matchers, &body, window));
    }
    Ok(results)
}

fn result(
    path: String,
    title: String,
    terms: &[String],
    matcher: &Matcher,
    term_matchers: &[Matcher],
    body: &[u32],
    window: MatchWindow,
) -> SearchResult {
    let file = Path::new(&path);
    let file_name = file.file_name().and_then(|value| value.to_str()).unwrap_or(&title);
    let match_count = matcher.find_all(file_name).len() + count::total(terms, body);
    let matched_terms = terms
        .iter()
        .zip(body)
        .zip(term_matchers)
        .filter_map(|((term, &count), term_matcher)| {
            (count > 0 || term_matcher.first(file_name).is_some()).then(|| term.clone())
        })
        .collect();
    let wanted = window.first.as_ref().map_or(0, |found| found.byte);
    let at = (0..=wanted.min(window.text.len())).rev().find(|&at| window.text.is_char_boundary(at)).unwrap_or(0);
    let heading = window.first.as_ref().and_then(|_| heading_at(&window.text, at));
    SearchResult {
        extension: file.extension().and_then(|value| value.to_str()).unwrap_or_default().to_owned(),
        snippet: excerpt(&window.text, at, window.ended),
        match_count,
        match_offset: window.first.as_ref().map_or(0, |found| found.utf16),
        heading,
        matched_terms,
        path,
        title,
    }
}
