use super::Counts;

#[cfg(test)]
mod benchmark;

// This immutable snapshot is built once per tree rebuild. Subtree totals then
// need two binary searches instead of walking all descendants for every row.
pub(crate) struct IndexedCounts<'a> {
    entries: Vec<(&'a str, u32)>,
    prefix: Vec<u64>,
}

impl Counts {
    pub(crate) fn indexed(&self) -> IndexedCounts<'_> {
        let mut prefix = Vec::with_capacity(self.own.len() + 1);
        prefix.push(0);
        let entries = self.own.iter().map(|(key, value)| {
            prefix.push(prefix.last().copied().unwrap_or(0) + u64::from(*value));
            (key.as_str(), *value)
        }).collect();
        IndexedCounts { entries, prefix }
    }
}

impl IndexedCounts<'_> {
    pub(crate) fn total(&self, key: &str) -> Option<u32> {
        let i = self.entries.binary_search_by(|(path, _)| path.cmp(&key)).ok()?;
        // Keep the exact same range boundaries as Counts::total: path siblings
        // such as 'dir-archive' and 'dir.other' are not children of 'dir'.
        let first_key = format!("{key}/");
        let after_key = format!("{key}0");
        let first = self.entries.partition_point(|(path, _)| *path < first_key.as_str());
        let after = self.entries.partition_point(|(path, _)| *path < after_key.as_str());
        let total = u64::from(self.entries[i].1) + self.prefix[after] - self.prefix[first];
        u32::try_from(total).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_totals_match_existing_range_sum_with_prefix_siblings() {
        let mut counts = Counts::default();
        for (key, value) in [
            ("c:/vault", 3),
            ("c:/vault-archive", 15),
            ("c:/vault.other", 22),
            ("c:/vault/a", 4),
            ("c:/vault/a/b", 5),
            ("c:/vault/a-1", 7),
            ("c:/vault/a-1/nested", 2),
            ("c:/vault/b", 6),
            ("c:/vault/бета", 2),
            ("c:/vault0", 99),
        ] {
            counts.own.insert(key.to_owned(), value);
        }
        let indexed = counts.indexed();
        for key in counts.own.keys() {
            let expected = counts.own[key] + counts.own.range(super::super::subtree(key))
                .map(|(_, value)| *value).sum::<u32>();
            assert_eq!(indexed.total(key), Some(expected), "{key}");
        }
        assert_eq!(indexed.total("c:/missing"), None);
    }

    #[test]
    fn refreshed_counts_match_full_scan_after_add_delete_and_rename() {
        let temp = crate::test_dir::tempdir();
        let root = temp.path();
        let nested = root.join("A/B");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("note.md"), "hello").unwrap();
        let mut counts = Counts::scan(root);
        let verify = |counts: &Counts| {
            let fresh = Counts::scan(root);
            assert_eq!(counts.own, fresh.own);
            let indexed = counts.indexed();
            for key in counts.own.keys() {
                assert_eq!(indexed.total(key), counts.total(std::path::Path::new(key)), "{key}");
            }
        };
        verify(&counts);
        std::fs::create_dir_all(root.join("Z/child")).unwrap();
        std::fs::write(root.join("Z/child/note.md"), "new").unwrap();
        counts.refresh(root);
        verify(&counts);
        std::fs::remove_dir_all(root.join("Z")).unwrap();
        counts.refresh(root);
        verify(&counts);
        std::fs::rename(root.join("A"), root.join("Renamed")).unwrap();
        counts.refresh(root);
        verify(&counts);
    }
}
