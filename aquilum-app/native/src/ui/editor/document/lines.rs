use std::ops::{Range, RangeInclusive};

pub(super) struct Lines {
    starts: Vec<usize>,
    len: usize,
}

fn starts(text: &str, base: usize) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(base).chain(text.match_indices('\n').map(move |(i, _)| base + i + 1))
}

impl Lines {
    pub(super) fn new(text: &str) -> Self {
        // Collect line starts in one scan. The previous exact reservation made
        // an extra full pass over the text just to count line breaks.
        let all = starts(text, 0).collect();
        Lines { starts: all, len: text.len() }
    }

    pub(super) fn count(&self) -> usize {
        self.starts.len()
    }

    pub(super) fn start(&self, line: usize) -> usize {
        self.starts[line]
    }

    pub(super) fn end(&self, line: usize) -> usize {
        self.starts.get(line + 1).map_or(self.len, |next| next - 1)
    }

    pub(super) fn range(&self, line: usize) -> Range<usize> {
        self.start(line)..self.end(line)
    }

    pub(super) fn at(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset).saturating_sub(1)
    }

    pub(super) fn splice(&mut self, lines: RangeInclusive<usize>, region: &str, delta: isize) -> usize {
        let fresh: Vec<usize> = starts(region, self.starts[*lines.start()]).collect();
        let count = fresh.len();
        for start in &mut self.starts[*lines.end() + 1..] {
            *start = start.wrapping_add_signed(delta);
        }
        self.starts.splice(lines, fresh);
        self.len = self.len.wrapping_add_signed(delta);
        count
    }

    pub(super) fn drop_front(&mut self, lines: usize, bytes: usize) {
        self.starts.drain(..lines);
        for start in &mut self.starts {
            *start -= bytes;
        }
        self.len -= bytes;
    }

    pub(super) fn truncate(&mut self, keep: usize, len: usize) {
        self.starts.truncate(keep);
        self.len = len;
    }

    pub(super) fn extend(&mut self, added: &str, at: usize) {
        self.starts.extend(added.match_indices('\n').map(|(i, _)| at + i + 1));
        self.len += added.len();
    }

    pub(super) fn prepend(&mut self, added: &str) -> usize {
        let mut all: Vec<usize> = starts(added, 0).collect();
        let fresh = all.len() - 1;
        all.extend(self.starts[1..].iter().map(|start| start + added.len()));
        self.starts = all;
        self.len += added.len();
        fresh
    }

    #[cfg(feature = "memory-probe")]
    pub(super) fn bytes(&self) -> usize {
        self.starts.capacity() * size_of::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::Lines;

    #[test]
    fn line_index_preserves_reference_byte_positions() {
        for text in [
            "", "a", "\n", "\n\n", "a\n", "a\nb", "a\r\nb\r\n",
            "Русский 😀 中文\nsecond\n", "\0\n\0", "line\n\nlast",
        ] {
            let expected: Vec<usize> = std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .collect();
            let lines = Lines::new(text);
            assert_eq!(lines.count(), expected.len(), "{text:?}");
            for (line, &at) in expected.iter().enumerate() {
                assert_eq!(lines.start(line), at, "{text:?}, line={line}");
                let end = expected.get(line + 1).map_or(text.len(), |next| next - 1);
                assert_eq!(lines.end(line), end, "{text:?}, line={line}");
                assert_eq!(lines.range(line), at..end, "{text:?}, line={line}");
            }
            for offset in 0..=text.len() {
                let expected_line = expected.partition_point(|&at| at <= offset).saturating_sub(1);
                assert_eq!(lines.at(offset), expected_line, "{text:?}, offset={offset}");
            }
        }
    }

    #[test]
    fn large_multilingual_line_index_matches_exact_reference() {
        let text = "Глава 😀 one\nline two\r\n\n漢字 text\n".repeat(4000);
        let lines = Lines::new(&text);
        let expected: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        assert_eq!(lines.count(), expected.len());
        for (line, &start) in expected.iter().enumerate() {
            assert_eq!(lines.start(line), start);
        }
    }
}
