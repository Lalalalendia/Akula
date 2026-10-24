use super::merge::TextEdit;
use std::fmt;
use yrs::updates::decoder::Decode;
use yrs::branch::{Branch, BranchPtr};
use yrs::updates::encoder::Encode;
use yrs::{
    Assoc, Doc, GetString, IndexedSequence, OffsetKind, Options, ReadTxn, StateVector, StickyIndex, Text, TextRef,
    Transact, Update,
};

const TEXT_NAME: &str = "codemirror";

#[derive(Debug)]
pub struct ReplicaError(String);

impl fmt::Display for ReplicaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn failure(error: impl fmt::Display) -> ReplicaError {
    ReplicaError(error.to_string())
}

/// Reject edits that would split a UTF-16 surrogate pair before Yrs is allowed
/// to mutate a document. Yrs 0.28.0 can otherwise remove the wrong character
/// or panic on an in-bounds, but non-scalar-aligned offset (upstream #714).
///
/// Validate all endpoints in one pass, including both endpoints of deletions.
/// The editor's change set is ordered and expressed in UTF-16 code units.
fn validate_utf16_edits(source: &str, edits: &[TextEdit]) -> Result<(), ReplicaError> {
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

pub struct Replica {
    doc: Doc,
    text: TextRef,
}

impl Replica {
    pub fn new() -> Self {
        let doc = Doc::with_options(Options { offset_kind: OffsetKind::Utf16, ..Options::default() });
        let text = doc.get_or_insert_text(TEXT_NAME);
        Self { doc, text }
    }

    pub fn restore(blobs: &[Vec<u8>]) -> Result<Self, ReplicaError> {
        let replica = Self::new();
        for blob in blobs {
            replica.apply_update(blob, "restore")?;
        }
        Ok(replica)
    }

    pub fn text(&self) -> String {
        self.text.get_string(&self.doc.transact())
    }

    pub fn len(&self) -> usize {
        self.text.len(&self.doc.transact()) as usize
    }

    pub fn apply_update(&self, update: &[u8], origin: &str) -> Result<(), ReplicaError> {
        let update = Update::decode_v1(update).map_err(failure)?;
        self.doc.transact_mut_with(origin).apply_update(update).map_err(failure)
    }

    pub fn apply_edits(&self, edits: &[TextEdit], origin: &str) -> Vec<u8> {
        let mut transaction = self.doc.transact_mut_with(origin);
        for edit in edits.iter().rev() {
            if edit.to > edit.from {
                self.text.remove_range(&mut transaction, edit.from as u32, (edit.to - edit.from) as u32);
            }
            if !edit.insert.is_empty() {
                self.text.insert(&mut transaction, edit.from as u32, &edit.insert);
            }
        }
        transaction.encode_update_v1()
    }

    /// Calculate an atomic Yrs delta without mutating the live editor replica.
    ///
    /// The temporary document uses the same client ID, but never edits in
    /// parallel with the live document. This prevents a new CRDT client
    /// identity from being introduced on every keystroke. The caller must
    /// persist the resulting delta before invoking apply_update on the live doc.
    /// This snapshot-based staging is correctness-first: benchmark/optimize
    /// large documents before the cloud sync executor is enabled.
    pub fn stage_edits(&self, batches: &[Vec<TextEdit>]) -> Result<Vec<u8>, ReplicaError> {
        let baseline = self.doc.transact().state_vector();
        let options = Options {
            client_id: self.doc.client_id(),
            offset_kind: OffsetKind::Utf16,
            ..Options::default()
        };
        let doc = Doc::with_options(options);
        let text = doc.get_or_insert_text(TEXT_NAME);
        let candidate = Self { doc, text };
        candidate.apply_update(&self.state(), "stage")?;
        for edits in batches {
            if !edits.is_empty() {
                // The candidate is still isolated: reject invalid UI input before
                // the session appends any delta to its durable SQLite journal.
                validate_utf16_edits(&candidate.text(), edits)?;
                candidate.apply_edits(edits, "stage");
            }
        }
        let update = candidate.doc.transact().encode_state_as_update_v1(&baseline);
        Ok(update)
    }

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

    pub fn state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }
}

#[cfg(test)]
#[path = "replica_tests.rs"]
mod tests;
