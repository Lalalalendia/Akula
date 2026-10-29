use std::borrow::Cow;

use super::split_line;
use super::super::{block, is_key};

// Lines borrow the original frontmatter until their value actually changes.
// Applying a batch must not clone the entire note for every updated field.
pub fn set_fields(text: &str, updates: &[(&str, String)]) -> Option<String> {
    // Preserve the original try_fold behavior for an empty batch, even if the
    // note does not have valid frontmatter.
    if updates.is_empty() {
        return Some(text.to_owned());
    }
    // Metadata fields are single-line. Refuse newlines instead of letting an
    // injected YAML fence rewrite the note body or subsequent updates.
    if updates.iter().any(|(key, value)| {
        key.trim().is_empty() || key.contains([':', '\r', '\n']) || value.contains(['\r', '\n'])
    }) {
        return None;
    }
    let block = block(text)?;
    let body = &text[block.body];
    let mut lines: Vec<Cow<'_, str>> = if body.is_empty() {
        Vec::new()
    } else {
        body.split('\n').map(Cow::Borrowed).collect()
    };

    // Only the short metadata header is revisited. The (potentially huge)
    // note body is appended once, after all sequential alias rules resolve.
    for (key, value) in updates {
        // The sequential setter reparses multiline values on each update.
        // Re-tokenize only metadata lines, never the note body.
        if lines.iter().any(|line| line.contains('\n')) {
            let mut physical = Vec::with_capacity(lines.len());
            for line in lines.drain(..) {
                if line.contains('\n') {
                    physical.extend(line.split('\n').map(|part| Cow::Owned(part.to_owned())));
                } else {
                    physical.push(line);
                }
            }
            lines = physical;
        }
        let mut found = false;
        lines.retain_mut(|line| {
            if !is_key(split_line(line.as_ref()).0, key) {
                return true;
            }
            found = true;
            if value.is_empty() {
                return false;
            }
            *line = Cow::Owned(format!("{key}: {value}"));
            true
        });
        if !found && !value.is_empty() {
            lines.push(Cow::Owned(format!("{key}: {value}")));
        }
    }

    let tail = &text[block.end..];
    let body_bytes = lines.iter().map(|line| line.len()).sum::<usize>() + lines.len().saturating_sub(1);
    let mut result = String::with_capacity(body_bytes + tail.len() + 8);
    result.push_str("---\n");
    for (index, line) in lines.iter().enumerate() {
        if index != 0 {
            result.push('\n');
        }
        result.push_str(line);
    }
    result.push_str("\n---");
    result.push_str(tail);
    Some(result)
}

// Keep the older single-field behavior exercised by the existing tests.
#[cfg(test)]
pub(super) fn set_field(text: &str, key: &str, value: &str) -> Option<String> {
    set_fields(text, &[(key, value.to_owned())])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontmatter::{BOOK_COVER, BOOK_FILE, COVER, PAGE_COVER, PAGE_COVER_POSITION, TYPE};

    // Original source algorithm: an independent oracle for output bytes,
    // update order, key aliases, removals, and failed frontmatter parsing.
    fn original_set_field(text: &str, key: &str, value: &str) -> Option<String> {
        let section = block(text)?;
        let body = &text[section.body];
        let mut found = false;
        let mut lines: Vec<String> = Vec::new();
        if !body.is_empty() {
            for line in body.split('\n') {
                if !is_key(split_line(line).0, key) {
                    lines.push(line.to_owned());
                    continue;
                }
                found = true;
                if !value.is_empty() {
                    lines.push(format!("{key}: {value}"));
                }
            }
        }
        if !found && !value.is_empty() {
            lines.push(format!("{key}: {value}"));
        }
        Some(format!("---\n{}\n---{}", lines.join("\n"), &text[section.end..]))
    }

    fn original_set_fields(text: &str, updates: &[(&str, String)]) -> Option<String> {
        updates.iter().try_fold(text.to_owned(), |current, (key, value)| {
            original_set_field(&current, key, value)
        })
    }

    fn next(seed: &mut u64, limit: usize) -> usize {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (*seed >> 32) as usize % limit
    }

    #[test]
    fn batch_matches_original_sequential_updates_for_all_edge_cases() {
        let samples = [
            "",
            "plain body without frontmatter",
            "---\n",
            "---\n---",
            "---\n\n---\n",
            "---\nCover: true\ncover: false\n---\nA\nB",
            "---\npage_cover_url: old\nPage_cover: newer\n---\nтело\n",
            "---\nBook_cover: a\ncover_url: b\nbook_file: old\n---\nтело",
            "---\nКлюч: значение\nİ: unicode\nkey without colon\n---\n正文",
            "---\ntype: book\ntags: [a, b]\n---\nemoji 🦈\r\nraw body",
            "---\r\ncover: true\r\n---\r\n",
        ];
        let keys = [
            COVER, TYPE, PAGE_COVER, BOOK_COVER, BOOK_FILE, PAGE_COVER_POSITION,
            "page_cover_url", "cover_url", "book_file", "PAGE_COVER_URL",
            "cover", "Cover", "tags", "Ключ", "İ", "key", "unknown",
        ];
        let values = ["", "true", "0", "pattern:weave", "Files/x.epub", "a:b", "[x, y]", "☺", r#" "x" "#, "more"];
        let mut seed = 0x2026_1022_f00d_u64;

        for sample in samples {
            for trial in 0..768 {
                let mut updates = Vec::new();
                for _ in 0..next(&mut seed, 12) {
                    updates.push((keys[next(&mut seed, keys.len())], values[next(&mut seed, values.len())].to_owned()));
                }
                let actual = set_fields(sample, &updates);
                let expected = original_set_fields(sample, &updates);
                assert_eq!(actual, expected, "trial={trial}, sample={sample:?}, updates={updates:?}");
            }
        }
    }

    #[test]
    fn rejects_multiline_keys_values_and_carriage_return_injections() {
        let initial = "---\ncover: old\n---\nbody";
        let cases = [
            vec![(COVER, "first\n---\ntail".to_owned()), (TYPE, "book".to_owned())],
            vec![(COVER, "first\n--- \t\ntail".to_owned()), (TYPE, "book".to_owned())],
            vec![("new\n---", "value".to_owned())],
            vec![(COVER, "new\r---\rbody".to_owned())],
            vec![("key\rbroken", "value".to_owned())],
            vec![("invalid:key", "value".to_owned())],
            vec![(" ", "value".to_owned())],
            vec![(COVER, "first\nsecond".to_owned())],
        ];
        for updates in cases {
            assert_eq!(set_fields(initial, &updates), None, "invalid update must fail closed");
        }
        assert_eq!(set_fields(initial, &[(COVER, "valid: value".to_owned())]).unwrap(),
            "---\ncover: valid: value\n---\nbody");
    }

    #[test]
    fn removing_and_readding_alias_preserves_original_order() {
        let initial = "---\npage_cover_url: a\nother: keep\n---\nbody";
        let updates = [
            (PAGE_COVER, "first".to_owned()),
            (PAGE_COVER, String::new()),
            (PAGE_COVER, "last".to_owned()),
        ];
        let output = set_fields(initial, &updates).unwrap();
        assert_eq!(output, "---\nother: keep\nPage_cover: last\n---\nbody");
        assert_eq!(Some(output), original_set_fields(initial, &updates));
    }

    #[test]
    fn large_unicode_body_is_preserved_with_many_updates() {
        let tail = "正文 🦈\r\n".repeat(32_768);
        let original = format!("---\ntype: book\nBook_cover: img.png\n---\n{tail}");
        let keys = [TYPE, BOOK_COVER, COVER, PAGE_COVER, BOOK_FILE];
        let updates = (0..20).map(|index| {
            (keys[index % keys.len()], format!("значение: {index}"))
        }).collect::<Vec<_>>();
        let updated = set_fields(&original, &updates).unwrap();
        assert!(updated.ends_with(&tail));
        assert_eq!(Some(updated), original_set_fields(&original, &updates));
    }

    #[test]
    fn empty_batch_keeps_non_frontmatter_notes_unchanged() {
        assert_eq!(set_fields("обычный текст", &[]), Some("обычный текст".to_owned()));
        assert_eq!(set_fields("обычный текст", &[("cover", "new".to_owned())]), None);
    }
}
