//! Aggregate, pre-inflation EPUB spine budget. The DOM cache is per spine index.
use super::{Zip, MAX_SECTION_BYTES};

const MAX_SPINE_SECTIONS: usize = 10_000;
const MAX_SPINE_TEXT_BYTES: usize = 512 * 1024 * 1024;

pub(super) fn valid(zip: &Zip, sections: &[(String, String)]) -> bool {
    if sections.is_empty() || sections.len() > MAX_SPINE_SECTIONS {
        return false;
    }
    let mut total = 0usize;
    sections.iter().all(|(path, _)| {
        let Some(size) = zip.size(path) else { return false };
        let Some(next) = total.checked_add(size) else { return false };
        total = next;
        !path.is_empty() && size <= MAX_SECTION_BYTES && total <= MAX_SPINE_TEXT_BYTES
    })
}

#[cfg(test)]
mod tests {
    use super::{valid, Zip, MAX_SPINE_SECTIONS, MAX_SECTION_BYTES};
    use crate::reader::session::epub_acceptance::synthetic_epub;

    fn archive() -> Zip {
        let (_dir, path) = synthetic_epub();
        Zip::new(std::fs::read(path).unwrap()).unwrap()
    }

    fn sections(count: usize) -> Vec<(String, String)> {
        std::iter::repeat_with(|| ("OPS/chapters/0.xhtml".to_owned(), String::new()))
            .take(count).collect()
    }

    #[test]
    fn normal_book_and_many_small_chapters_remain_eligible() {
        let zip = archive();
        assert!(valid(&zip, &sections(3)));
        assert!(valid(&zip, &sections(MAX_SPINE_SECTIONS)));
        assert!(!valid(&zip, &sections(MAX_SPINE_SECTIONS + 1)));
        assert!(!valid(&zip, &[]));
    }

    #[test]
    fn aggregate_declared_size_rejected_before_inflation() {
        let (_dir, path) = synthetic_epub();
        let mut bytes = std::fs::read(path).unwrap();
        let needle = b"OPS/chapters/0.xhtml";
        let central = 0x0201_4b50u32.to_le_bytes();
        let at = bytes.windows(4).enumerate().find_map(|(at, sig)| {
            (sig == central && bytes.get(at + 46..at + 46 + needle.len()) == Some(needle))
                .then_some(at)
        }).unwrap();
        bytes[at + 24..at + 28].copy_from_slice(&(MAX_SECTION_BYTES as u32).to_le_bytes());
        let zip = Zip::new(bytes).unwrap();
        assert!(valid(&zip, &sections(16))); // Exactly 512 MiB.
        assert!(!valid(&zip, &sections(17))); // 544 MiB, no inflate.
    }

    #[test]
    fn missing_or_overlong_section_rejected() {
        let zip = archive();
        assert!(!valid(&zip, &[("OPS/missing.xhtml".to_owned(), String::new())]));
        assert!(!valid(&zip, &[(String::new(), String::new())]));
    }
}
