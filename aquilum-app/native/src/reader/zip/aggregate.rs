//! Bound the advertised uncompressed size of accessible ZIP entries.
use super::Zip;

impl Zip {
    pub(in crate::reader) fn declared_total_under(&self, limit: usize) -> bool {
        self.entries.values().try_fold(0usize, |sum, entry| {
            let next = sum.checked_add(entry.size)?;
            (next <= limit).then_some(next)
        }).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::Zip;
    use crate::reader::epub::Epub;
    use crate::reader::session::epub_acceptance::synthetic_epub;

    #[test]
    fn sum_of_all_accessible_entries_honors_exact_limit() {
        let (_dir, path) = synthetic_epub();
        let bytes = std::fs::read(path).unwrap();
        let archive = Zip::new(bytes.clone()).unwrap();
        let declared: usize = archive.entries.values().map(|entry| entry.size).sum();
        assert!(archive.declared_total_under(declared));
        assert!(!archive.declared_total_under(declared - 1));
        assert!(Epub::open(bytes).is_some());
    }

    #[test]
    fn huge_non_spine_member_is_rejected_without_decompression() {
        let (_dir, path) = synthetic_epub();
        let mut bytes = std::fs::read(path).unwrap();
        let signature = 0x0201_4b50u32.to_le_bytes();
        let at = bytes.windows(4).enumerate().find_map(|(at, chunk)| {
            (chunk == signature && bytes.get(at + 46..at + 54) == Some(&b"mimetype"[..]))
                .then_some(at)
        }).expect("mimetype central entry");
        // The mimetype is not part of the spine: old chapter-only checks admit this.
        bytes[at + 24..at + 28].copy_from_slice(&(1_073_741_825u32).to_le_bytes());
        assert!(Zip::new(bytes.clone()).is_some(), "archive metadata should still parse");
        assert!(Epub::open(bytes).is_none(), "entry sum must fail before any inflate");
    }
}
