//! Keep generated frontmatter fields on single physical YAML lines.
const LINE_BREAKS: [char; 5] = ['\r', '\n', '\u{0085}', '\u{2028}', '\u{2029}'];

fn quote(value: &str) -> String {
    // JSON double-quoted scalars are also valid YAML quoted scalars.
    // serde_json escapes CR/LF; YAML also treats NEL/LS/PS as line breaks.
    serde_json::to_string(value).expect("serializing a string cannot fail")
        .replace('\u{0085}', "\\u0085")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub(super) fn render(fields: &[(&str, &str)]) -> String {
    let mut out = String::from("---\n");
    for &(key, value) in fields {
        let quoted_key = (key.is_empty() || key.contains(':')
            || key.contains(LINE_BREAKS) || key.starts_with(['#', '?', '-', '!']));
        let quoted_value = value.contains(LINE_BREAKS)
            || value.contains(": ") || value.contains(" #");
        if quoted_key { out.push_str(&quote(key)); } else { out.push_str(key); }
        out.push_str(": ");
        if quoted_value { out.push_str(&quote(value)); } else { out.push_str(value); }
        out.push('\n');
    }
    out.push_str("---\n");
    out
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::frontmatter::{end, parse, AUTHOR, COVER, TAGS, TYPE};

    #[test]
    fn preserves_existing_simple_fields_and_yaml_lists() {
        let expected = "---\ntags: []\nauthor: \n---\n";
        assert_eq!(render(&[(TAGS, "[]"), (AUTHOR, "")]), expected);
        assert_eq!(parse(expected).unwrap().text(AUTHOR), None);
    }

    #[test]
    fn injected_yaml_fence_is_only_a_quoted_scalar() {
        let value = "hello\n---\ntype: attacker\n---\nbody";
        let actual = render(&[(COVER, value), (TYPE, "book")]);
        assert!(actual.contains(r#"cover: "hello\n---\ntype: attacker\n---\nbody""#));
        assert!(actual.contains("\ntype: book\n---\n"));
        assert_eq!(actual.lines().filter(|line| *line == "---").count(), 2);
        assert_eq!(end(&actual), Some(actual.len() - 1));
    }

    #[test]
    fn controls_and_injected_keys_do_not_create_new_yaml_lines() {
        for (key, value) in [
            ("title\n---\nother", "safe"),
            ("description", "line\r\n---\r\nbody"),
            ("unicode", "first\u{0085}second\u{2028}third\u{2029}last"),
            ("invalid:key", "hello"),
            ("description", "valid: with comment # unsafe"),
        ] {
            let actual = render(&[(key, value)]);
            assert_eq!(actual.lines().count(), 3, "unexpected extra lines: {actual:?}");
            assert_eq!(actual.lines().filter(|line| *line == "---").count(), 2);
            assert!(parse(&actual).is_some());
        }
    }
}
