use super::{Fold, Folded, Matcher, Mode};
use std::ops::Range;

/// The common one-needle, ASCII-only case avoids per-character Unicode
/// folding. All other input retains the reference Unicode scanner.
pub(super) fn each(
    matcher: &Matcher,
    text: &str,
    found: &mut impl FnMut(Range<usize>) -> bool,
) -> bool {
    if matcher.needles.is_empty() {
        return true;
    }
    if matcher.needles.len() == 1
        && matcher.needles[0].iter().all(char::is_ascii)
        && text.is_ascii()
    {
        return ascii_each(matcher, text, found);
    }
    unicode_each(matcher, text, found)
}

fn ascii_each(
    matcher: &Matcher,
    text: &str,
    found: &mut impl FnMut(Range<usize>) -> bool,
) -> bool {
    let bytes = text.as_bytes();
    let needle = &matcher.needles[0];
    let first = needle[0] as u8;
    let fold_case = matcher.options.fold != Fold::Exact;
    let alternate = if fold_case { first.to_ascii_uppercase() } else { first };
    let mut cursor = 0;
    loop {
        let rest = &bytes[cursor..];
        let relative = if first == alternate {
            memchr::memchr(first, rest)
        } else {
            memchr::memchr2(first, alternate, rest)
        };
        let Some(relative) = relative else {
            return true;
        };
        let start = cursor + relative;
        let end = start + needle.len();
        let starts_word = matcher.options.mode == Mode::Substring
            || start == 0
            || !(bytes[start - 1] as char).is_alphanumeric();
        let ends_word = matcher.options.mode != Mode::WholeWord
            || end == bytes.len()
            || (end < bytes.len() && !(bytes[end] as char).is_alphanumeric());
        let matches = end <= bytes.len()
            && needle.iter().enumerate().all(|(offset, wanted)| {
                let current = bytes[start + offset];
                let folded = if fold_case { current.to_ascii_lowercase() } else { current };
                folded == *wanted as u8
            });
        if starts_word && ends_word && matches {
            if !found(start..end) {
                return false;
            }
            cursor = end;
        } else {
            cursor = start + 1;
        }
    }
}

/// Reference scanner: keep its exact Unicode, folding and boundary behavior.
pub(super) fn unicode_each(
    matcher: &Matcher,
    text: &str,
    found: &mut impl FnMut(Range<usize>) -> bool,
) -> bool {
    if matcher.needles.is_empty() {
        return true;
    }
    let mut rest = Folded::new(text, matcher.options.fold);
    let mut before = None;
    while let Some((first, start, end)) = rest.next() {
        if matcher.starts_word(before) {
            if let Some((end, after, last)) = matcher.match_at(first, end, &rest) {
                if !found(start..end) {
                    return false;
                }
                (rest, before) = (after, Some(last));
                rest.skip_rest_of_char();
                continue;
            }
        }
        before = Some(first);
    }
    true
}
