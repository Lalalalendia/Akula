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
    if has_body_match {
        if let Some(phrase) = quoted_matcher {
            if let Ok(window) = match_and_read_through(
                file,
                Some(phrase),
                CHARS_AFTER_MATCH * 4,
                cancel,
            ) {
                if window.first.is_some() {
                    return window;
                }
            }
        }
    }
    match match_and_read_through(
        file,
        has_body_match.then_some(matcher),
        CHARS_AFTER_MATCH * 4,
        cancel,
    ) {
        Ok(window) => window,
        Err(_) if has_body_match && !cancel.is_cancelled() => {
            // Keep the previous bounded opening-snippet fallback on scan errors.
            match_and_read_through(file, None, CHARS_AFTER_MATCH * 4, cancel)
                .unwrap_or_else(|_| empty_window())
        }
        Err(_) => empty_window(),
    }
}

fn empty_window() -> MatchWindow {
    MatchWindow { first: None, text: String::new(), ended: true }
}
