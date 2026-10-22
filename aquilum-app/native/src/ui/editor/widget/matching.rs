use std::ops::Range;

use aquilum_core::text::cancel::Cancel;
use aquilum_core::text::matcher::{EXACT, IGNORE_CASE, Matcher, Mode, Options};

pub(super) const FIND: Options = Options { limit: 19_999, ..IGNORE_CASE };
pub(super) const SELECTION: Options = EXACT;
pub(super) const HOVER: Options = Options { mode: Mode::WordPrefix, ..IGNORE_CASE };

/// Ctrl+F supports literal alternatives separated by ||; \|| matches literal pipes.
pub(super) fn find_terms(query: &str) -> Vec<String> {
    let mut chars = query.chars().peekable();
    let (mut parts, mut part, mut split) = (Vec::new(), String::new(), false);
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if chars.peek() == Some(&'\\') {
                chars.next();
                part.push('\\');
                continue;
            }
            if chars.peek() == Some(&'|') {
                chars.next();
                if chars.peek() == Some(&'|') {
                    chars.next();
                    part.push_str("||");
                    continue;
                }
                part.push('\\');
                part.push('|');
                continue;
            }
        }
        if ch == '|' && chars.peek() == Some(&'|') {
            chars.next();
            parts.push(std::mem::take(&mut part));
            split = true;
        } else {
            part.push(ch);
        }
    }
    parts.push(part);
    if split {
        parts.into_iter().map(|part| part.trim().to_owned()).filter(|part| !part.is_empty()).collect()
    } else {
        parts.into_iter().filter(|part| !part.is_empty()).collect()
    }
}

pub(super) fn find_in(text: &crop::Rope, matcher: &Matcher, cancel: &Cancel) -> Vec<Range<usize>> {
    let mut lines = matcher.lines(cancel);
    crate::rope::each_line(text.byte_slice(..), |line| lines.feed(line));
    lines.finish()
}

#[derive(Clone, Debug, Default)]
pub struct Search {
    pub query: String,
    pub only: Option<usize>,
}

impl Search {
    pub fn ranges(&self, text: &str, seen: &mut usize) -> Vec<Range<usize>> {
        let found = Matcher::any(&find_terms(&self.query), FIND).find_all(text);
        let first = *seen;
        *seen += found.len();
        match self.only {
            None => found,
            Some(k) => found.into_iter().enumerate().filter(|(i, _)| first + i == k).map(|(_, r)| r).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FIND, find_in, find_terms, Search};
    use aquilum_core::text::cancel::Cancel;
    use aquilum_core::text::matcher::Matcher;

    #[test]
    fn the_whole_rope_is_searched_line_by_line() {
        let text: String = (0..3000).map(|i| format!("строка {i} Текст\n")).collect();
        let matcher = Matcher::new("текст", FIND);
        let found = find_in(&crop::Rope::from(text.as_str()), &matcher, &Cancel::never());
        assert_eq!(found.len(), 3000);
        assert_eq!(found, matcher.find_all(&text));
    }
    #[test]
    fn search_query_parses_or_and_literal_pipes() {
        assert_eq!(find_terms(" перевод || переводчик ||  "), ["перевод", "переводчик"]);
        assert_eq!(find_terms(r"\||"), ["||"]);
        assert_eq!(find_terms(" фраза "), [" фраза "]);
        assert_eq!(find_terms(" || "), Vec::<String>::new());
        assert_eq!(find_terms("один || || два"), ["один", "два"]);
    }

    #[test]
    fn or_search_is_leftmost_longest_and_shared_with_embeds() {
        let text = "переводчик перевод Переводчик";
        let query = "перевод || переводчик";
        let expected = ["переводчик", "перевод", "Переводчик"];
        let found = Matcher::any(&find_terms(query), FIND).find_all(text);
        assert_eq!(found.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), expected);
        let mut seen = 0;
        assert_eq!(Search { query: query.into(), only: None }.ranges(text, &mut seen), found);
        let repeated = Matcher::any(&find_terms("aa || a"), FIND).find_all("aaaa");
        assert_eq!(repeated, [0..2, 2..4]);
    }

}
