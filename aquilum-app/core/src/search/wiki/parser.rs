use super::target::{is_attachment, parts};
use crate::text::utf16::utf16_len;
use std::ops::Range;

mod excluded;
use excluded::excluded_ranges;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WikiLink {
    pub target: String,
    pub note_range: Range<usize>,
    pub offset_utf16: usize,
}

struct Found {
    start: usize,
    note_range: Range<usize>,
    target: String,
}

pub fn extract(source: &str) -> Vec<WikiLink> {
    // Most prose-only notes contain no wiki links. Avoid allocating the
    // Markdown parser's excluded ranges until a link opener is possible.
    if !source.contains("[[") {
        return Vec::new();
    }
    scan_candidates(source)
}

fn scan_candidates(source: &str) -> Vec<WikiLink> {
    let excluded = excluded_ranges(source);
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut cursor = 0;
    let mut excluded_at = 0;
    // Search directly for the two-byte opener instead of branching on each
    // byte of prose. Keep the old overlap handling for escaped openers.
    let opener = memchr::memmem::Finder::new(b"[[");
    while cursor + 3 < bytes.len() {
        let Some(next) = opener.find(&bytes[cursor..]) else {
            break;
        };
        cursor += next;
        if cursor + 3 >= bytes.len() {
            break;
        }
        if escaped(bytes, cursor) {
            cursor += 1;
            continue;
        }
        let Some(end) = find_close(bytes, cursor + 2) else {
            cursor += 2;
            continue;
        };
        let full = cursor..end + 2;
        while excluded_at < excluded.len() && excluded[excluded_at].end <= full.start {
            excluded_at += 1;
        }
        let inside_code = excluded
            .get(excluded_at)
            .is_some_and(|range| overlaps(range, &full));
        let embed = cursor > 0 && bytes[cursor - 1] == b'!' && !escaped(bytes, cursor - 1);
        let inner = &source[cursor + 2..end];
        let split = parts(inner);
        let note = &inner[split.note.clone()];
        let skipped = inside_code || note.is_empty() || (embed && is_attachment(note));
        if !skipped {
            let target_end = split.anchor.as_ref().map_or(split.note.end, |anchor| anchor.end);
            found.push(Found {
                start: cursor,
                note_range: cursor + 2 + split.note.start..cursor + 2 + split.note.end,
                target: inner[split.note.start..target_end].to_owned(),
            });
        }
        cursor = end + 2;
    }
    let mut prior = 0;
    let mut offset_utf16 = 0;
    found
        .into_iter()
        .map(|link| {
            offset_utf16 += utf16_len(&source[prior..link.start]);
            prior = link.start;
            WikiLink {
                target: link.target,
                note_range: link.note_range,
                offset_utf16,
            }
        })
        .collect()
}
fn find_close(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    while cursor + 1 < bytes.len() {
        if bytes[cursor] == b'\n' || bytes[cursor] == b'\r' {
            return None;
        }
        if bytes[cursor] == b']' && bytes[cursor + 1] == b']' && !escaped(bytes, cursor) {
            return Some(cursor);
        }
        cursor += 1;
    }
    None
}

fn escaped(bytes: &[u8], at: usize) -> bool {
    let mut count = 0;
    let mut cursor = at;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        count += 1;
        cursor -= 1;
    }
    count % 2 == 1
}

fn overlaps(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

#[cfg(test)]
mod tests {
    use super::extract;

    #[test]
    fn fast_path_is_identical_to_the_full_markdown_scanner() {
        for source in [
            "",
            "Plain text and *emphasis* with no link opener",
            "Title [single bracket] and escaped \\[ sequences",
            "Текст 😀 с UTF-16 символами",
            "[[Valid]] `[[Ignored]]`",
            "```md\n[[Hidden]]\n```",
            "[[unclosed and [[Valid]]",
        ] {
            assert_eq!(super::extract(source), super::scan_candidates(source));
        }
    }

    #[test]
    #[ignore = "run explicitly on the wiki parser benchmark workflow"]
    fn benchmark_no_wikilinks_fast_path() {
        use std::hint::black_box;
        use std::time::Instant;

        let prose = (0..2500)
            .map(|i| format!("## Section {i}\nA prose paragraph with a [markdown label](https://example.invalid/{i}) and no wiki links.\n"))
            .collect::<String>();
        assert!(super::extract(&prose).is_empty());
        let measure = |scanner: fn(&str) -> Vec<super::WikiLink>| {
            let mut samples = Vec::new();
            for _ in 0..9 {
                let start = Instant::now();
                for _ in 0..8 {
                    black_box(scanner(black_box(prose.as_str())));
                }
                samples.push(start.elapsed().as_secs_f64() * 1_000.0 / 8.0);
            }
            samples.sort_by(f64::total_cmp);
            samples[samples.len() / 2]
        };
        let slow = measure(super::scan_candidates);
        let fast = measure(super::extract);
        println!(
            "WIKI_PERF case=no_openers paragraphs=2500 original_median_ms={slow:.4} optimized_median_ms={fast:.4} ratio={:.2}",
            slow / fast
        );
    }

    fn targets(source: &str) -> Vec<String> {
        extract(source).into_iter().map(|link| link.target).collect()
    }

    #[test]
    fn extracts_targets_and_ignores_code() {
        let source = "[[Note]] [[Folder/Name|alias]] `[[Code]]`\n```\n[[Block]]\n```";
        assert_eq!(targets(source), ["Note", "Folder/Name"]);
    }

    #[test]
    fn continues_after_an_unclosed_link() {
        let links = extract("[[broken\n[[Note]]");
        assert_eq!(links[0].target, "Note");
    }

    #[test]
    fn keeps_the_anchor_in_the_target_but_not_in_the_note_range() {
        let source = "см. [[Заметка#Заголовок|подпись]]";
        let link = &extract(source)[0];
        assert_eq!(link.target, "Заметка#Заголовок");
        assert_eq!(&source[link.note_range.clone()], "Заметка");
        assert_eq!(link.offset_utf16, "см. ".encode_utf16().count());
    }

    #[test]
    fn links_to_the_current_note_are_not_edges() {
        assert!(extract("[[#Заголовок]] и [[ #^abc ]]").is_empty());
    }

    #[test]
    fn table_cell_alias_does_not_leak_a_backslash_into_the_target() {
        let source = r"| [[Note\|alias]] | [[Folder/Note#H\|a]] |";
        let links = extract(source);
        assert_eq!(links.iter().map(|link| link.target.as_str()).collect::<Vec<_>>(), ["Note", "Folder/Note#H"]);
        assert_eq!(&source[links[0].note_range.clone()], "Note");
    }

    #[test]
    fn embedded_files_are_not_links_but_embedded_notes_are() {
        let source = "![[image.png|610]] ![[document.pdf#page=3]] ![[Заметка]] ![[Заметка#Раздел]] [[image.png]]";
        assert_eq!(targets(source), ["Заметка", "Заметка#Раздел", "image.png"]);
    }

    #[test]
    fn escaped_bang_keeps_a_plain_link() {
        assert_eq!(targets(r"\![[image.png]]"), ["image.png"]);
    }
}
