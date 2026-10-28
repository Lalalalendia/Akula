use super::merge::TextEdit;
use std::fmt;
use yrs::updates::encoder::Encode;
use yrs::{
    Doc, GetString, IndexedSequence, OffsetKind, Options, ReadTxn, StateVector, Text, TextRef, Transact,
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

#[path = "replica_utf16.rs"]
mod utf16_boundary;
#[path = "replica_update_guard.rs"]
mod update_guard;
#[path = "replica_update_decoder.rs"]
mod update_decoder;
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
        let update = update_guard::decode(update)?;
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
    /// The temporary document reuses its client ID without concurrent edits,
    /// avoiding a new CRDT identity per keystroke. Persist the delta durably
    /// before applying it to the live document. Correctness-first: benchmark
    /// large documents before enabling cloud sync.
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
                utf16_boundary::validate_utf16_edits(&candidate.text(), edits)?;
            }
            candidate.apply_edits(edits, "stage");
        }
        let update = candidate.doc.transact().encode_state_as_update_v1(&baseline);
        Ok(update)
    }

    pub fn state(&self) -> Vec<u8> {
        self.doc.transact().encode_state_as_update_v1(&StateVector::default())
    }
}

#[cfg(feature = "canvas-crdt-lab")]
#[path = "canvas_crdt_lab.rs"]
pub mod canvas_crdt_lab;

#[cfg(test)]
#[path = "replica_tests.rs"]
mod tests;
