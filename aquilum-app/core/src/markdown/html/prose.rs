//! Escaping pasted HTML text without accidentally creating Markdown syntax.
pub fn prose(text: &str, before: Option<char>, line_start: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let blank = |c: Option<&char>| c.is_none_or(|c| c.is_whitespace());
    let mut out = String::with_capacity(text.len());
    let mut previous = before;
    for (at, &c) in chars.iter().enumerate() {
        let next = chars.get(at + 1);
        let first = line_start && at == 0;
        let escaped = match c {
            '*' | '`' | '[' | ']' => true,
            '\\' => next.is_none_or(char::is_ascii_punctuation),
            '_' => !previous.is_some_and(char::is_alphanumeric) || !next.is_some_and(|n| n.is_alphanumeric()),
            '~' => previous == Some('~') || next == Some(&'~'),
            '<' => next.is_some_and(|n| n.is_alphabetic() || matches!(n, '/' | '!' | '?')),
            '&' => {
                let name = chars[at + 1..].iter().take_while(|n| n.is_alphanumeric() || **n == '#').count();
                name > 0 && chars.get(at + 1 + name) == Some(&';')
            }
            '#' => first && blank(chars.iter().find(|&&n| n != '#')),
            '>' => first,
            '-' | '+' => first && blank(next),
            '.' | ')' => line_start && at > 0 && chars[..at].iter().all(char::is_ascii_digit) && blank(next),
            _ => false,
        };
        if escaped {
            out.push('\\');
        }
        out.push(c);
        previous = Some(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::prose;
    #[test]
    fn block_markers_and_inline_markup_are_escaped() {
        assert_eq!(prose("1. list", None, true), r"1\. list");
        assert_eq!(prose("- item", None, true), r"\- item");
        assert_eq!(prose("a * b", None, false), r"a \* b");
    }
}
