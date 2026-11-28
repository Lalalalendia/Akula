//! Private validation primitives shared by the unpublished provider crates.
use subtle::ConstantTimeEq;

pub(super) fn valid_uuid(raw: &str) -> bool {
    let b = raw.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, ch)| {
        if [8, 13, 18, 23].contains(&i) { *ch == b'-' }
        else { ch.is_ascii_digit() || (b'a'..=b'f').contains(ch) }
    })
}

pub(super) fn valid_label(label: &str, max: usize) -> bool {
    !label.is_empty() && label.len() <= max &&
        label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub(super) fn valid_token(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && text.bytes().all(|b| b.is_ascii_graphic())
}

pub(super) fn matches_readback(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && bool::from(a.ct_eq(b))
}
