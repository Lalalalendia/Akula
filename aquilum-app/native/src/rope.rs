use crop::{Rope, RopeSlice};

pub fn floor(text: &Rope, at: usize) -> usize {
    let mut at = at.min(text.byte_len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

pub fn prefix(text: &Rope, len: usize) -> String {
    text.byte_slice(..floor(text, len)).to_string()
}

pub fn each_line(text: RopeSlice<'_>, mut line: impl FnMut(&str) -> bool) {
    let mut part = String::new();
    for chunk in text.chunks() {
        let mut rest = chunk;
        while let Some(i) = rest.find('\n') {
            let go = if part.is_empty() {
                line(&rest[..i])
            } else {
                part.push_str(&rest[..i]);
                let go = line(&part);
                part.clear();
                go
            };
            if !go {
                return;
            }
            rest = &rest[i + 1..];
        }
        part.push_str(rest);
    }
    line(&part);
}

pub fn common_prefix(a: &Rope, b: &Rope) -> usize {
    let (mut left, mut right) = (a.chunks().map(str::as_bytes), b.chunks().map(str::as_bytes));
    let (mut x, mut y): (&[u8], &[u8]) = (&[], &[]);
    let mut same = 0;
    loop {
        if x.is_empty() {
            match left.next() {
                Some(chunk) => x = chunk,
                None => return same,
            }
            continue;
        }
        if y.is_empty() {
            match right.next() {
                Some(chunk) => y = chunk,
                None => return same,
            }
            continue;
        }
        let n = x.len().min(y.len());
        // Most incremental edits share entire rope chunks. Equality of slices
        // uses the optimized byte-compare path for those chunks; only a
        // differing chunk needs a scalar search for its exact byte offset.
        if x[..n] != y[..n] {
            let offset = x[..n]
                .iter()
                .zip(&y[..n])
                .position(|(a, b)| a != b)
                .expect("unequal slices must contain a different byte");
            return same + offset;
        }
        same += n;
        (x, y) = (&x[n..], &y[n..]);
    }
}

#[cfg(test)]
mod tests {
    use super::{common_prefix, each_line};
    use crop::Rope;

    #[test]
    fn common_prefix_crosses_chunks() {
        let text: String = (0..5000).map(|i| format!("строка {i}\n")).collect();
        let (a, mut b) = (Rope::from(text.as_str()), Rope::from(text.as_str()));
        assert_eq!(common_prefix(&a, &b), text.len());
        b.insert(30_000, "x");
        assert_eq!(common_prefix(&a, &b), 30_000);
        assert_eq!(common_prefix(&a, &Rope::new()), 0);
    }

    #[test]
    fn common_prefix_reports_raw_byte_offsets_for_multibyte_codepoints() {
        let a = Rope::from("a😀z");
        let b = Rope::from("a😃z");
        let flat = "a😀z".as_bytes().iter().zip("a😃z".as_bytes())
            .take_while(|(x, y)| x == y).count();
        assert_eq!(common_prefix(&a, &b), flat);
        assert_eq!(common_prefix(&b, &a), flat);
    }

    #[test]
    fn common_prefix_matches_flat_bytes_for_chunk_boundaries_and_edits() {
        let source = "ASCII Русский 漢字 😀!\n".repeat(2400);
        let a = Rope::from(source.as_str());
        let positions = [
            0, 1, 7, 30, 127, 1024, 4096, source.len() / 2,
            source.len().saturating_sub(40), source.len(),
        ];
        for position in positions {
            if !source.is_char_boundary(position) {
                continue;
            }
            let mut changed = source.clone();
            changed.insert_str(position, "Z");
            let b = Rope::from(changed.as_str());
            let expected = source.bytes().zip(changed.bytes())
                .take_while(|(left, right)| left == right).count();
            assert_eq!(common_prefix(&a, &b), expected, "insertion at {position}");
            assert_eq!(common_prefix(&b, &a), expected, "reverse insertion at {position}");
        }
        let short = &source[..source.char_indices().nth(100).unwrap().0];
        assert_eq!(common_prefix(&a, &Rope::from(short)), short.len());
        assert_eq!(common_prefix(&a, &Rope::from(source.as_str())), source.len());
    }

    #[test]
    fn each_line_matches_split() {
        let text: String = (0..3000).map(|i| format!("строка {i}\n")).collect::<String>() + "хвост";
        let mut lines = Vec::new();
        each_line(Rope::from(text.as_str()).byte_slice(..), |line| {
            lines.push(line.to_owned());
            true
        });
        assert_eq!(lines, text.split('\n').collect::<Vec<_>>());
    }
}
