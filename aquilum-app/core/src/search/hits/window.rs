use super::excerpt::CHARS_AFTER_MATCH;
use super::file::{match_and_read_through, MatchWindow};
use crate::text::cancel::Cancel;
use crate::text::matcher::Matcher;
use std::path::Path;

pub(super) fn read_window(
    file: &Path,
    has_body_match: bool,
    matcher: &Matcher,
    quoted_matcher: Option<&Matcher>,
    cancel: &Cancel,
) -> MatchWindow {
        let preferred = match (has_body_match, quoted_matcher) {
            (true, Some(phrase)) => match_and_read_through(
                file, Some(phrase), CHARS_AFTER_MATCH * 4, cancel,
            ).ok().filter(|window| window.first.is_some()),
            _ => None,
        };
        let window = match preferred.map(Ok::<MatchWindow, std::io::Error>).unwrap_or_else(|| match_and_read_through(
            file,
            has_body_match.then_some(matcher),
            CHARS_AFTER_MATCH * 4,
            cancel,
        )) {
            Ok(window) => window,
            Err(_) if has_body_match && !cancel.is_cancelled() => {
                // Match scan errors previously fell back to an opening snippet
                // on a second open. Retain that behavior only on this error path.
                match_and_read_through(file, None, CHARS_AFTER_MATCH * 4, cancel)
                    .unwrap_or_else(|_| empty_window())
            }
            Err(_) => empty_window(),
        };
    window
}

fn empty_window() -> MatchWindow {
    MatchWindow { first: None, text: String::new(), ended: true }
}

