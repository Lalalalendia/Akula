//! UTF-16 scalar-boundary checks before passing edits to Yrs 0.28.0.
//! Upstream #714: split-surrogate positions can mutate the wrong character or panic.
use super::{failure, Replica, ReplicaError, TextEdit};
use yrs::branch::{Branch, BranchPtr};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Assoc, IndexedSequence, StickyIndex, Text, Transact};

pub(super) fn validate_utf16_edits(source: &str, edits: &[TextEdit]) -> Result<(), ReplicaError> {
    if edits.is_empty() {
        return Ok(());
    }

    let mut endpoints = Vec::with_capacity(edits.len().saturating_mul(2));
    let mut previous_end = 0;
    for edit in edits {
        if edit.from < previous_end || edit.from > edit.to {
            return Err(failure("overlapping or descending UTF-16 edits"));
        }
        if edit.to > u32::MAX as usize {
            return Err(failure("UTF-16 edit offset exceeds Yrs limit"));
        }
        endpoints.extend([edit.from, edit.to]);
        previous_end = edit.to;
    }

    endpoints.sort_unstable();
    endpoints.dedup();
    let mut position = 0;
    let mut next_endpoint = 0;
    for character in source.chars() {
        while next_endpoint < endpoints.len() && endpoints[next_endpoint] == position {
            next_endpoint += 1;
        }
        position += character.len_utf16();
        if next_endpoint < endpoints.len() && endpoints[next_endpoint] < position {
            return Err(failure("UTF-16 edit splits a surrogate pair"));
        }
    }
    while next_endpoint < endpoints.len() && endpoints[next_endpoint] == position {
        next_endpoint += 1;
    }
    if next_endpoint != endpoints.len() {
        return Err(failure("UTF-16 edit extends beyond the document"));
    }
    Ok(())
}

impl Replica {
    pub fn encode_position(&self, index: usize) -> Option<Vec<u8>> {
        let transaction = self.doc.transact();
        let index = index.min(self.text.len(&transaction) as usize) as u32;
        self.text.sticky_index(&transaction, index, Assoc::After).map(|position| position.encode_v1())
    }

    pub fn resolve_position(&self, encoded: &[u8]) -> Option<usize> {
        let position = StickyIndex::decode_v1(encoded).ok()?;
        let transaction = self.doc.transact();
        let offset = position.get_offset(&transaction)?;
        let text: &Branch = self.text.as_ref();
        (offset.branch == BranchPtr::from(text)).then_some(offset.index as usize)
    }
}
