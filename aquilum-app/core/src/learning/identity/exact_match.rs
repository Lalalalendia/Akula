//! Exact, unique evidence matching for learning row and table reconciliation.
//! Callers retain source validation, digest domains, and public error types.
use std::collections::BTreeMap;

pub(in crate::learning) struct EvidenceIndex<K> {
    old: BTreeMap<K, usize>,
    new: BTreeMap<K, Vec<usize>>,
}
impl<K: Ord + Copy> EvidenceIndex<K> {
    pub(in crate::learning) fn new(previous: impl IntoIterator<Item = K>, current: &[K]) -> Self {
        let mut old = BTreeMap::<K, usize>::new();
        for key in previous { *old.entry(key).or_default() += 1; }
        let mut new = BTreeMap::<K, Vec<usize>>::new();
        for (index, &key) in current.iter().enumerate() {
            new.entry(key).or_default().push(index);
        }
        Self { old, new }
    }

    pub(in crate::learning) fn find<E>(&self, key: &K, missing: E, ambiguous: E) -> Result<usize, E> {
        let count = self.old.get(key).copied().unwrap_or(0);
        let candidates = self.new.get(key).map(Vec::as_slice).unwrap_or(&[]);
        if count == 1 && candidates.len() == 1 {
            Ok(candidates[0])
        } else if count > 1 || candidates.len() > 1 {
            Err(ambiguous)
        } else {
            Err(missing)
        }
    }

    pub(in crate::learning) fn unbound(&self, current: &[K], offset: usize) -> Vec<usize> {
        current.iter().enumerate().filter_map(|(index, key)| {
            self.find(key, (), ()).is_err().then_some(index + offset)
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_requires_uniqueness_on_both_sides_and_preserves_index_base() {
        let current = [4, 3, 3, 2, 9, 9];
        let matches = EvidenceIndex::new([1, 2, 2, 3, 4], &current);
        assert_eq!(matches.find(&1, "missing", "ambiguous"), Err("missing"));
        assert_eq!(matches.find(&2, "missing", "ambiguous"), Err("ambiguous"));
        assert_eq!(matches.find(&3, "missing", "ambiguous"), Err("ambiguous"));
        assert_eq!(matches.find(&4, "missing", "ambiguous"), Ok(0));
        assert_eq!(matches.find(&9, "missing", "ambiguous"), Err("ambiguous"));
        assert_eq!(matches.unbound(&current, 0), vec![1, 2, 3, 4, 5]);
        assert_eq!(matches.unbound(&current, 1), vec![2, 3, 4, 5, 6]);
    }
}
