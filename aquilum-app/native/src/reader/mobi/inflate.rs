//! Streaming-state PalmDOC expansion with a per-book output budget.
pub(super) fn decompress_bounded(src: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(src.len().saturating_mul(2).min(limit).min(64 * 1024));
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        i += 1;
        match b {
            0x00 => {
                if out.len() == limit { return None; }
                out.push(0);
            }
            0x01..=0x08 => {
                let count = b as usize;
                let end = i.saturating_add(count).min(src.len());
                if end - i > limit - out.len() { return None; }
                out.extend_from_slice(&src[i..end]);
                i = i.saturating_add(count);
            }
            0x09..=0x7F => {
                if out.len() == limit { return None; }
                out.push(b);
            }
            0x80..=0xBF => {
                if i < src.len() {
                    let b2 = src[i]; i += 1;
                    let pair = ((b as usize) << 8 | b2 as usize) & 0x3FFF;
                    let dist = pair >> 3;
                    let len = (pair & 7) + 3;
                    if dist > 0 && dist <= out.len() {
                        if len > limit - out.len() { return None; }
                        let start = out.len() - dist;
                        if dist >= len {
                            out.extend_from_within(start..start + len);
                        } else {
                            for j in 0..len {
                                let byte = out[start + (j % dist)];
                                out.push(byte);
                            }
                        }
                    }
                }
            }
            0xC0..=0xFF => {
                if 2 > limit - out.len() { return None; }
                out.push(b' ');
                out.push(b ^ 0x80);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::decompress_bounded;

    #[test]
    fn refuses_growth_over_output_budget_without_losing_normal_palmdoc() {
        let input = b"abcd\x80\x21\xe1";
        assert_eq!(decompress_bounded(input, 10), Some(b"abcdabcd a".to_vec()));
        assert_eq!(decompress_bounded(input, 9), None);
        assert_eq!(decompress_bounded(input, 0), None);
        assert_eq!(decompress_bounded(b"", 0), Some(Vec::new()));
    }

    #[test]
    fn rejects_repetitive_backrefs_before_expansion() {
        let mut input = vec![b'A'];
        input.extend(std::iter::repeat_n([0x80u8, 0x0f]).take(10_000).flatten());
        assert!(decompress_bounded(&input, 100).is_none());
        assert_eq!(decompress_bounded(&input, 100_001).unwrap().len(), 100_001);
    }
}
