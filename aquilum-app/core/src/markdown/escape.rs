const SPECIAL: [char; 10] = ['*', '_', '`', '|', '<', '>', '~', '\\', '[', ']'];

pub fn label(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(next) if c == '\\' && next.is_ascii_punctuation() => {
                out.push(c);
                out.extend(chars.next());
            }
            _ => {
                if SPECIAL.contains(&c) {
                    out.push('\\');
                }
                out.push(c);
            }
        }
    }
    out
}

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
    use super::label;

    #[test]
    fn markup_characters_are_escaped_once() {
        assert_eq!(label("Текст [с] *скобками*"), r"Текст \[с\] \*скобками\*");
        assert_eq!(label(r"уже \*экранировано\*"), r"уже \*экранировано\*");
        assert_eq!(label(r"C:\Users"), r"C:\\Users");
        assert_eq!(label("конец\\"), "конец\\\\");
    }

    #[test]
    fn escaping_twice_changes_nothing() {
        for text in ["a*b_c", r"x\y", "[1](2)", "<tag> | ~", r"\\", ""] {
            let once = label(text);
            assert_eq!(label(&once), once, "{text}");
        }
    }
}
