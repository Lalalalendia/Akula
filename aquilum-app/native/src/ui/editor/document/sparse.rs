use std::collections::HashMap;
use std::ops::RangeInclusive;

pub(super) struct Sparse<T>(HashMap<usize, T>);

impl<T> Default for Sparse<T> {
    fn default() -> Self {
        Sparse(HashMap::new())
    }
}

impl<T> Sparse<T> {
    pub(super) fn get(&self, line: usize) -> Option<&T> {
        self.0.get(&line)
    }

    pub(super) fn get_mut(&mut self, line: usize) -> Option<&mut T> {
        self.0.get_mut(&line)
    }

    pub(super) fn insert(&mut self, line: usize, value: T) {
        self.0.insert(line, value);
    }

    pub(super) fn remove(&mut self, line: usize) {
        self.0.remove(&line);
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }

    pub(super) fn retain(&mut self, keep: impl Fn(usize) -> bool) {
        self.0.retain(|line, _| keep(*line));
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, &T)> {
        self.0.iter().map(|(line, value)| (*line, value))
    }

    pub(super) fn shift(&mut self, by: usize) {
        self.0 = std::mem::take(&mut self.0).into_iter().map(|(line, value)| (line + by, value)).collect();
    }

    pub(super) fn splice(&mut self, lines: RangeInclusive<usize>, count: usize) {
        let (first, last) = (*lines.start(), *lines.end());
        // Replacing N lines with N lines leaves every later cache key intact.
        // Avoid reallocating/rehashing all visible paragraph and embed entries
        // on the overwhelmingly common one-line typing path.
        if count > 0 && first <= last && last - first == count - 1 {
            if first == last {
                self.0.remove(&first);
            } else {
                // A large replacement range should cost O(cached entries), not
                // O(number of document lines in the selected range).
                self.0.retain(|line, _| !lines.contains(line));
            }
            return;
        }
        self.0 = std::mem::take(&mut self.0)
            .into_iter()
            .filter(|(line, _)| !lines.contains(line))
            .map(|(line, value)| (if line > last { line - last - 1 + first + count } else { line }, value))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::Sparse;
    use std::collections::HashMap;
    use std::ops::RangeInclusive;

    #[test]
    fn one_line_edit_invalidates_only_that_line_without_rebuilding_cache() {
        let mut cache = Sparse::<usize>::default();
        for line in 0..64 {
            cache.insert(line, line + 10_000);
        }
        let capacity = cache.0.capacity();
        let untouched = cache.get(40).unwrap() as *const usize;
        for value in 0..128 {
            cache.splice(13..=13, 1);
            assert_eq!(cache.get(13), None);
            assert_eq!(cache.get(12), Some(&(12 + 10_000)));
            assert_eq!(cache.get(14), Some(&(14 + 10_000)));
            assert_eq!(cache.get(40), Some(&(40 + 10_000)));
            cache.insert(13, value);
        }
        assert_eq!(cache.0.capacity(), capacity);
        assert!(std::ptr::eq(untouched, cache.get(40).unwrap() as *const usize));
    }

    #[test]
    fn same_count_multiline_edit_invalidates_only_the_replaced_lines() {
        let mut cache = Sparse::<usize>::default();
        for line in 0..40 {
            cache.insert(line, line);
        }
        cache.splice(6..=11, 6);
        assert_eq!(cache.0.len(), 34);
        for line in 0..40 {
            assert_eq!(cache.get(line).copied(), (!(6..=11).contains(&line)).then_some(line));
        }
    }

    #[test]
    fn growing_shrinking_and_empty_replacements_remap_unaffected_keys() {
        for (range, replacement_count, expected) in [
            (4..=4, 3, vec![(0, 0), (1, 1), (2, 2), (3, 3), (7, 5), (8, 6), (9, 7), (10, 8)]),
            (3..=6, 1, vec![(0, 0), (1, 1), (2, 2), (4, 7), (5, 8)]),
            (3..=4, 0, vec![(0, 0), (1, 1), (2, 2), (3, 5), (4, 6), (5, 7), (6, 8)]),
        ] {
            let mut cache = Sparse::<usize>::default();
            for line in 0..9 {
                cache.insert(line, line);
            }
            cache.splice(range, replacement_count);
            let actual: HashMap<usize, usize> = cache.iter().map(|(k, value)| (k, *value)).collect();
            let expected: HashMap<usize, usize> = expected.into_iter().collect();
            assert_eq!(actual, expected);
        }
    }

    fn original_splice(cache: &mut HashMap<usize, usize>, lines: RangeInclusive<usize>, count: usize) {
        let (first, last) = (*lines.start(), *lines.end());
        *cache = std::mem::take(cache)
            .into_iter()
            .filter(|(line, _)| !lines.contains(line))
            .map(|(line, value)| (if line > last { line - last - 1 + first + count } else { line }, value))
            .collect();
    }

    #[test]
    fn deterministic_differential_cases_match_original_splice() {
        let mut seed = 0xA51_7B33_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as usize
        };
        for case in 0..2_048 {
            let mut cache = Sparse::<usize>::default();
            let mut original = HashMap::new();
            for _ in 0..(next() % 60) {
                let (key, value) = (next() % 140, next());
                cache.insert(key, value);
                original.insert(key, value);
            }
            let first = next() % 100;
            let last = first + next() % 12;
            let count = next() % 14;
            let range = first..=last;
            cache.splice(range.clone(), count);
            original_splice(&mut original, range, count);
            let result: HashMap<usize, usize> = cache.iter().map(|(k, value)| (k, *value)).collect();
            assert_eq!(result, original, "mismatch in synthetic edit {case}");
        }
    }

    #[test]
    fn very_long_equal_count_range_does_not_iterate_unpopulated_document_lines() {
        let mut cache = Sparse::<usize>::default();
        cache.insert(0, 7);
        cache.insert(200, 8);
        cache.insert(usize::MAX, 9);
        cache.splice(1..=usize::MAX - 1, usize::MAX - 1);
        assert_eq!(cache.get(0), Some(&7));
        assert_eq!(cache.get(200), None);
        assert_eq!(cache.get(usize::MAX), Some(&9));
    }
}
