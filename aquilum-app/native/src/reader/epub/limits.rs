//! Conservative per-entry inflation caps for untrusted EPUB archives.
//! The ZIP payload is still held in memory by the existing Book::open API.
use super::Zip;

pub(super) const MAX_PACKAGE_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_SECTION_BYTES: usize = 32 * 1024 * 1024;
pub(super) const MAX_RESOURCE_BYTES: usize = 32 * 1024 * 1024;

pub(super) fn valid_spine(zip: &Zip, sections: &[(String, String)]) -> bool {
    super::aggregate::valid(zip, sections)
}

#[cfg(test)]
mod tests {
    use crate::reader::{Book, session::epub_acceptance::synthetic_epub};

    const CHAPTER: &[u8] = b"OPS/chapters/0.xhtml";

    fn change_declared_size(bytes: &mut [u8], name: &[u8], mutate: impl FnOnce(u32) -> u32) {
        let central_signature = 0x0201_4b50u32.to_le_bytes();
        let at = bytes
            .windows(4)
            .enumerate()
            .find_map(|(at, signature)| {
                (signature == central_signature
                    && bytes.get(at + 46..at + 46 + name.len()) == Some(name))
                .then_some(at)
            })
            .expect("fixture member in central directory");
        let size_offset = at + 24;
        let original = u32::from_le_bytes(
            bytes[size_offset..size_offset + 4].try_into().unwrap(),
        );
        bytes[size_offset..size_offset + 4].copy_from_slice(&mutate(original).to_le_bytes());
    }

    #[test]
    fn rejects_oversized_declared_epub_section_before_inflation() {
        let (_dir, path) = synthetic_epub();
        let mut bytes = std::fs::read(&path).unwrap();
        change_declared_size(&mut bytes, CHAPTER, |_| 33 * 1024 * 1024);
        std::fs::write(&path, bytes).unwrap();
        assert!(Book::open(&path).is_none());
    }

    #[test]
    fn rejects_truncated_stored_section_without_rendering_empty_content() {
        let (_dir, path) = synthetic_epub();
        let mut bytes = std::fs::read(&path).unwrap();
        change_declared_size(&mut bytes, CHAPTER, |size| size + 1);
        std::fs::write(&path, bytes).unwrap();
        let book = Book::open(&path).expect("declared section is still below the size cap");
        assert!(book.document(0).is_none());
    }

    #[test]
    fn normal_synthetic_epub_still_opens_all_chapters() {
        let (_dir, path) = synthetic_epub();
        let book = Book::open(&path).expect("valid EPUB");
        assert_eq!(book.sections.len(), 3);
        for index in 0..3 {
            assert!(book.document(index).is_some());
        }
    }
}
