use std::cell::OnceCell;
use super::epub::{self, Epub};
use super::{Book, Section};

pub fn open(data: Vec<u8>) -> Option<Book> {
    if data.len() < 78 {
        return None;
    }
    let num_records = u16::from_be_bytes([data[76], data[77]]) as usize;
    if num_records == 0 || data.len() < 78 + num_records * 8 {
        return None;
    }

    let mut record_offsets = Vec::with_capacity(num_records);
    for i in 0..num_records {
        let base = 78 + i * 8;
        let offset = u32::from_be_bytes([data[base], data[base + 1], data[base + 2], data[base + 3]]) as usize;
        record_offsets.push(offset);
    }

    // Проверяем, есть ли встроенный EPUB ZIP (стандартный формат AZW3/KF8)
    for &offset in &record_offsets {
        if offset + 4 <= data.len() && &data[offset..offset + 4] == b"PK\x03\x04" {
            let zip_data = data[offset..].to_vec();
            if let Some(epub) = Epub::open(zip_data) {
                let sections = epub.sections.iter().map(|(_, base)| Section { base: base.clone(), dom: OnceCell::new() }).collect();
                return Some(Book { epub: Some(epub), binaries: std::collections::HashMap::new(), sections });
            }
        }
    }

    let rec0_offset = record_offsets[0];
    let rec0_end = if num_records > 1 { record_offsets[1].min(data.len()) } else { data.len() };
    if rec0_offset >= data.len() || rec0_end <= rec0_offset {
        return None;
    }
    let rec0 = &data[rec0_offset..rec0_end];
    if rec0.len() < 12 {
        return None;
    }

    let compression = u16::from_be_bytes([rec0[0], rec0[1]]);
    let record_count = (u16::from_be_bytes([rec0[8], rec0[9]]) as usize).min(num_records.saturating_sub(1));
    let header_len = rec0.get(20..24).map_or(0, |b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let trailing = if header_len >= 228 { rec0.get(242..244).map_or(0, |b| u16::from_be_bytes([b[0], b[1]])) } else { 0 };

    let mut decompressed = Vec::new();
    for i in 1..=record_count {
        let start = record_offsets[i];
        let end = if i + 1 < num_records { record_offsets[i + 1].min(data.len()) } else { data.len() };
        if start >= data.len() || end <= start {
            continue;
        }
        let slice = strip_trailing(&data[start..end], trailing);
        match compression {
            1 => decompressed.extend_from_slice(slice),
            2 => decompressed.extend_from_slice(&decompress_palmdoc(slice)),
            _ => return None,
        }
    }

    if decompressed.is_empty() {
        return None;
    }

    let dom = epub::document(&decompressed).or_else(|| {
        let mut wrapped = Vec::with_capacity(decompressed.len() + 100);
        wrapped.extend_from_slice(b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body>");
        wrapped.extend_from_slice(&decompressed);
        wrapped.extend_from_slice(b"</body></html>");
        epub::document(&wrapped)
    })?;

    let sections = vec![Section { base: "epubcfi(/6/2)".to_owned(), dom: OnceCell::from(Some(dom)) }];
    Some(Book { epub: None, binaries: std::collections::HashMap::new(), sections })
}

fn strip_trailing(mut record: &[u8], flags: u16) -> &[u8] {
    for _ in 0..(flags >> 1).count_ones() {
        let tail = &record[record.len().saturating_sub(4)..];
        let size = tail.iter().fold(0usize, |value, &b| if b & 0x80 != 0 { usize::from(b & 0x7f) } else { value << 7 | usize::from(b & 0x7f) });
        record = &record[..record.len().saturating_sub(size)];
    }
    if flags & 1 != 0
        && let Some(&last) = record.last()
    {
        record = &record[..record.len().saturating_sub(usize::from(last & 3) + 1)];
    }
    record
}

fn decompress_palmdoc(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 2);
    let mut i = 0;
    while i < src.len() {
        let b = src[i];
        i += 1;
        match b {
            0x00 => out.push(0),
            0x01..=0x08 => {
                let count = b as usize;
                let end = (i + count).min(src.len());
                out.extend_from_slice(&src[i..end]);
                i += count;
            }
            0x09..=0x7F => out.push(b),
            0x80..=0xBF => {
                if i < src.len() {
                    let b2 = src[i];
                    i += 1;
                    let pair = ((b as usize) << 8 | b2 as usize) & 0x3FFF;
                    let dist = pair >> 3;
                    let len = (pair & 7) + 3;
                    if dist > 0 && dist <= out.len() {
                        let start = out.len() - dist;
                        if dist >= len {
                            out.extend_from_within(start..start + len);
                        } else {
                            // Overlapping references repeat bytes from their own output.
                            for j in 0..len {
                                let byte = out[start + (j % dist)];
                                out.push(byte);
                            }
                        }
                    }
                }
            }
            0xC0..=0xFF => {
                out.push(b' ');
                out.push(b ^ 0x80);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn palmdoc_back_reference_uses_eleven_bit_distance() {
        assert_eq!(super::decompress_palmdoc(&[b'a', b'b', b'c', b'd', 0x80, 0x21, 0xE1]), b"abcdabcd a");
    }

    #[test]
    fn trailing_entries_are_cut_like_foliate() {
        assert_eq!(super::strip_trailing(b"text\x01\x02\x83", 0b10), b"text");
        assert_eq!(super::strip_trailing(b"text\x09\x02\x83", 0b11), b"tex");
        assert_eq!(super::strip_trailing(b"text", 0), b"text");
    }

    #[test]
    fn palmdoc_all_back_reference_distances_and_lengths() {
        for distance in 1usize..=2047 {
            let seed: Vec<u8> = (0..distance).map(|i| b'A' + (i % 26) as u8).collect();
            for length in 3usize..=10 {
                let pair = 0x8000 | (distance << 3) | (length - 3);
                let mut input = seed.clone();
                input.extend_from_slice(&[(pair >> 8) as u8, pair as u8]);
                let mut expected = seed.clone();
                expected.extend(seed.iter().copied().cycle().take(length));
                assert_eq!(super::decompress_palmdoc(&input), expected, "distance={distance}, length={length}");
            }
        }
    }

    #[test]
    fn palmdoc_reference_uses_tail_and_supports_chained_overlap() {
        // Copy the last three bytes, then repeat the last byte ten times.
        assert_eq!(super::decompress_palmdoc(b"abcdef\x80\x18\x80\x0f"), b"abcdefdefffffffffff");
    }

    #[test]
    fn palmdoc_preserves_invalid_reference_and_truncated_input_behavior() {
        // Invalid references are skipped, not a reason to discard the remaining text.
        assert_eq!(super::decompress_palmdoc(b"abc\x80\x00Z"), b"abcZ");
        assert_eq!(super::decompress_palmdoc(b"abc\x80\x20Z"), b"abcZ");
        assert_eq!(super::decompress_palmdoc(b"abc\x80"), b"abc");
        assert_eq!(super::decompress_palmdoc(&[8, b'a', b'b']), b"ab");
        assert_eq!(super::decompress_palmdoc(&[0, 0xc1, 0xff]), &[0, b' ', b'A', b' ', 0x7f]);
    }
}
