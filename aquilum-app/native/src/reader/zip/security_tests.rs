use super::{Zip, MAX_LEGACY_READ_BYTES};

fn fixture() -> Vec<u8> {
    let (_dir, path) = crate::reader::session::epub_acceptance::synthetic_epub();
    std::fs::read(path).unwrap()
}

fn central_entry(bytes: &[u8], name: &[u8]) -> usize {
    bytes.windows(4).enumerate().find_map(|(at, part)| {
        (part == 0x0201_4b50u32.to_le_bytes()
            && bytes.get(at + 46..at + 46 + name.len()) == Some(name)).then_some(at)
    }).expect("central directory entry")
}

#[test]
fn legacy_zip_read_rejects_oversized_claim_without_allocating_it() {
    let mut bytes = fixture();
    let at = central_entry(&bytes, b"OPS/chapters/0.xhtml");
    let huge = (MAX_LEGACY_READ_BYTES as u32 + 1).to_le_bytes();
    bytes[at + 24..at + 28].copy_from_slice(&huge);
    let archive = Zip::new(bytes).expect("metadata may be parsed without inflation");
    assert!(archive.read("OPS/chapters/0.xhtml").is_none());
    assert!(archive.read_limited("OPS/chapters/0.xhtml", 1024).is_none());
}

#[test]
fn malformed_central_offset_fails_without_unchecked_access() {
    let mut bytes = fixture();
    let eocd = bytes.windows(4).rposition(|part| part == 0x0605_4b50u32.to_le_bytes())
        .expect("end of central directory");
    bytes[eocd + 16..eocd + 20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Zip::new(bytes).is_none());
}

#[test]
fn valid_epub_remains_compatible_with_limited_zip_read() {
    let bytes = fixture();
    let archive = Zip::new(bytes).unwrap();
    assert!(archive.read("META-INF/container.xml").is_some());
    assert!(archive.read_limited("META-INF/container.xml", 4 * 1024 * 1024).is_some());
}
