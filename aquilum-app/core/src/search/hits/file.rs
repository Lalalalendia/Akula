use crate::files::document::normalize_line_endings_in_place;
use crate::text::cancel::Cancel;
use crate::text::matcher::Matcher;
use crate::text::utf16::utf16_len;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub struct FirstMatch {
    pub byte: usize,
    pub utf16: usize,
}

#[derive(Debug)]
pub struct MatchWindow {
    pub first: Option<FirstMatch>,
    pub text: String,
    pub ended: bool,
}

struct NoteLines {
    reader: BufReader<File>,
    bytes: Vec<u8>,
}

impl NoteLines {
    fn open(path: &Path) -> io::Result<Self> {
        Ok(Self { reader: BufReader::new(File::open(path)?), bytes: Vec::new() })
    }

    fn next(&mut self) -> io::Result<Option<&str>> {
        self.bytes.clear();
        if self.reader.read_until(b'\n', &mut self.bytes)? == 0 {
            return Ok(None);
        }
        let kept = normalize_line_endings_in_place(&mut self.bytes);
        self.bytes.truncate(kept);
        std::str::from_utf8(&self.bytes)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    fn rewind(&mut self) -> io::Result<()> {
        self.reader.seek(SeekFrom::Start(0))?;
        self.bytes.clear();
        Ok(())
    }
}

fn read_through_lines(lines: &mut NoteLines, at: usize, after: usize) -> io::Result<(String, bool)> {
    let mut text = String::new();
    let mut lines_after = 0;
    while text.len() < at.saturating_add(after) || lines_after < 2 {
        let Some(line) = lines.next()? else {
            return Ok((text, true));
        };
        text.push_str(line);
        if text.len() > at {
            lines_after += 1;
        }
    }
    let ended = lines.reader.fill_buf()?.is_empty();
    Ok((text, ended))
}

/// Read a search-result window through one file handle. A hit in the opening
/// window is processed in one forward pass. For distant hits, seek the same
/// handle back to reconstruct the prefix required by heading_at().
///
/// Until a hit is found, retain at most the normal opening snippet (apart
/// from a single potentially long line). This prevents an outdated index
/// from buffering an entire large file that no longer contains the term.
/// Text remains a prefix: heading_at and excerpt require its original offsets.
pub fn match_and_read_through(
    path: &Path,
    matcher: Option<&Matcher>,
    after: usize,
    cancel: &Cancel,
) -> io::Result<MatchWindow> {
    let mut lines = NoteLines::open(path)?;
    let Some(matcher) = matcher else {
        let (text, ended) = read_through_lines(&mut lines, 0, after)?;
        return Ok(MatchWindow { first: None, text, ended });
    };

    let mut prefix = String::new();
    let mut prefix_lines = 0;
    let mut prefix_complete = false;
    let mut prefix_ended = false;
    let (mut byte, mut utf16) = (0usize, 0usize);

    while let Some(line) = lines.next()? {
        if let Some(found) = matcher.first(line) {
            let before = &line[..found.start];
            let first = FirstMatch {
                byte: byte + before.len(),
                utf16: utf16 + utf16_len(before),
            };
            if prefix_complete {
                // A distant match: replay only on the same pinned file handle.
                // No unbounded prefix allocation occurs for stale-index misses.
                lines.rewind()?;
                let (text, ended) = read_through_lines(&mut lines, first.byte, after)?;
                return Ok(MatchWindow { first: Some(first), text, ended });
            }

            prefix.push_str(line);
            let mut lines_after = usize::from(prefix.len() > first.byte);
            while prefix.len() < first.byte.saturating_add(after) || lines_after < 2 {
                let Some(next) = lines.next()? else {
                    return Ok(MatchWindow { first: Some(first), text: prefix, ended: true });
                };
                prefix.push_str(next);
                if prefix.len() > first.byte {
                    lines_after += 1;
                }
            }
            let ended = lines.reader.fill_buf()?.is_empty();
            return Ok(MatchWindow { first: Some(first), text: prefix, ended });
        }

        if cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "search cancelled"));
        }
        // Finish using the borrowed line before probing the reader's buffer.
        byte += line.len();
        utf16 += utf16_len(line);
        if !prefix_complete {
            prefix.push_str(line);
            if !prefix.is_empty() {
                prefix_lines += 1;
            }
            if prefix.len() >= after && prefix_lines >= 2 {
                prefix_complete = true;
                prefix_ended = lines.reader.fill_buf()?.is_empty();
            }
        }
    }
    Ok(MatchWindow { first: None, text: prefix, ended: !prefix_complete || prefix_ended })
}

// Keep the old two-open implementation only for differential tests and the
// same-process benchmark; it is not part of production search.
#[cfg(test)]
fn first_match(path: &Path, matcher: &Matcher, cancel: &Cancel) -> io::Result<Option<FirstMatch>> {
    let mut lines = NoteLines::open(path)?;
    let (mut byte, mut utf16) = (0, 0);
    while let Some(line) = lines.next()? {
        if let Some(found) = matcher.first(line) {
            let before = &line[..found.start];
            return Ok(Some(FirstMatch { byte: byte + before.len(), utf16: utf16 + utf16_len(before) }));
        }
        if cancel.is_cancelled() {
            return Ok(None);
        }
        (byte, utf16) = (byte + line.len(), utf16 + utf16_len(line));
    }
    Ok(None)
}

#[cfg(test)]
fn read_through(path: &Path, at: usize, after: usize) -> io::Result<(String, bool)> {
    read_through_lines(&mut NoteLines::open(path)?, at, after)
}

#[cfg(test)]
mod tests {
    use super::{first_match, match_and_read_through, read_through, MatchWindow};
    use crate::search::hits::{query_matcher, CHARS_AFTER_MATCH};
    use crate::text::cancel::Cancel;
    use crate::text::matcher::Matcher;
    use std::fs;
    use std::path::Path;

    const AFTER: usize = CHARS_AFTER_MATCH * 4;

    fn legacy(path: &Path, matcher: Option<&Matcher>) -> MatchWindow {
        let first = matcher.and_then(|m| first_match(path, m, &Cancel::never()).unwrap());
        let at = first.as_ref().map_or(0, |hit| hit.byte);
        let (text, ended) = read_through(path, at, AFTER).unwrap();
        MatchWindow { first, text, ended }
    }

    fn compare(bytes: &[u8], term: Option<&str>) -> MatchWindow {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.md");
        fs::write(&path, bytes).unwrap();
        let matcher = term.map(|value| query_matcher(&[value]));
        let reference = legacy(&path, matcher.as_ref());
        let actual = match_and_read_through(&path, matcher.as_ref(), AFTER, &Cancel::never()).unwrap();
        assert_eq!(actual.first, reference.first);
        assert_eq!(actual.text, reference.text);
        assert_eq!(actual.ended, reference.ended);
        actual
    }

    #[test]
    fn single_handle_preserves_early_and_title_only_snippets() {
        compare(b"# Title\n\nsearch appears now\nand has more text\n", Some("search"));
        let window = compare(b"Only a title match\nNothing in the body\n", None);
        assert!(window.first.is_none());
        assert!(window.ended);
    }

    #[test]
    fn single_handle_preserves_utf16_crlf_and_far_matches() {
        let mut text = "# Глава\r\n".to_owned();
        text.push_str(&"😀 строка без совпадений\r\n".repeat(400));
        text.push_str("Наконец ТЕСТирование\r\nи продолжение\r\n");
        let window = compare(text.as_bytes(), Some("тест"));
        assert!(window.first.unwrap().utf16 > 400);
        assert!(window.text.contains("ТЕСТирование"));
    }

    #[test]
    fn single_handle_does_not_buffer_a_stale_index_miss() {
        let text = "Обычная строка без совпадения\n".repeat(30_000);
        let window = compare(text.as_bytes(), Some("необнаружимый"));
        assert!(window.first.is_none());
        assert!(window.text.len() < 2_000, "a stale index miss must not buffer the whole file");
        assert!(!window.ended, "the opening excerpt is truncated");
    }

    #[test]
    fn single_handle_preserves_eof_and_no_match_offsets() {
        compare(b"", Some("test"));
        compare(b"one line", Some("test"));
        compare(b"one line", None);
        compare(b"first line\nsecond line\nthird line\n", Some("absent"));
        compare("Строка\rбез совпадения\n".as_bytes(), Some("неизвестно"));
    }

    #[test]
    #[ignore = "diagnostic same-process benchmark; run in release mode"]
    fn benchmark_search_hit_window() {
        use std::hint::black_box;
        use std::time::Instant;

        fn sample(path: &Path, matcher: &Matcher, baseline: bool, iterations: usize) -> f64 {
            let start = Instant::now();
            for _ in 0..iterations {
                let window = if baseline {
                    legacy(path, Some(matcher))
                } else {
                    match_and_read_through(path, Some(matcher), AFTER, &Cancel::never()).unwrap()
                };
                black_box((window.first.map(|hit| hit.byte), window.text.len(), window.ended));
            }
            start.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64
        }

        let directory = tempfile::tempdir().unwrap();
        for (label, body, iterations) in [
            ("early", "term at the start\n".to_owned() + &"more lines\n".repeat(500), 200),
            ("distant", "not here yet\n".repeat(5000) + "finally term appears\n", 30),
            ("stale", "not here\n".repeat(5000), 30),
        ] {
            let path = directory.path().join(format!("{label}.md"));
            fs::write(&path, body).unwrap();
            let matcher = query_matcher(&["term"]);
            let mut old = Vec::new();
            let mut new = Vec::new();
            for _ in 0..4 {
                old.push(sample(&path, &matcher, true, iterations));
                new.push(sample(&path, &matcher, false, iterations));
                new.push(sample(&path, &matcher, false, iterations));
                old.push(sample(&path, &matcher, true, iterations));
            }
            old.sort_by(f64::total_cmp);
            new.sort_by(f64::total_cmp);
            let baseline_us = (old[3] + old[4]) / 2.0;
            let candidate_us = (new[3] + new[4]) / 2.0;
            println!(
                "BENCH-HITS case={label} baseline_us={baseline_us:.3} candidate_us={candidate_us:.3} speedup={:.3}x",
                baseline_us / candidate_us
            );
        }
    }
}
