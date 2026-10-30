use std::ops::Range;

use super::Document;

impl Document {
    pub fn slice(&self, range: Range<usize>) -> &str {
        range.start.checked_sub(self.base).zip(range.end.checked_sub(self.base)).and_then(|(a, b)| self.text.get(a..b)).unwrap_or("")
    }

    pub fn before(&self, at: usize) -> &str {
        at.checked_sub(self.base).and_then(|end| self.text.get(..end)).unwrap_or("")
    }

    pub fn after(&self, at: usize) -> &str {
        at.checked_sub(self.base).and_then(|start| self.text.get(start..)).unwrap_or("")
    }

    pub fn loaded(&self) -> (usize, &str) {
        (self.base, &self.text)
    }

    pub fn end(&self) -> usize {
        self.base + self.text.len()
    }

    pub fn loaded_range(&self) -> (usize, usize) {
        (self.base, self.end())
    }

    pub fn include(&mut self, at: usize) {
        if at < self.base || at > self.end() {
            self.load(at, None);
        }
    }

    pub fn find(&self, pattern: &str) -> Option<usize> {
        self.text.find(pattern).map(|at| self.base + at)
    }

    pub fn search(&self, find: impl Fn(&str) -> Vec<Range<usize>>) -> Vec<Range<usize>> {
        find(&self.text).into_iter().map(|found| self.base + found.start..self.base + found.end).collect()
    }
}
