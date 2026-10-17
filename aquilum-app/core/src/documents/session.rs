use super::changes::{edits_from_json, edits_to_json, length_after, ChangeError};
use super::conflict::{ConflictCause, ConflictReport};
use super::merge::{merge_external_change, TextEdit};
use super::replica::{Replica, ReplicaError};
use super::resolve::{resolve_sync, SyncInputs, SyncPoint, SyncPointLookup, SyncVerdict};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::fmt;

const LOG_LIMIT: usize = 1024;
pub const DISK_CLIENT: &str = "disk";
pub const SYSTEM_CLIENT: &str = "system";

pub struct DiskSnapshot {
    pub content: String,
    pub hash: String,
    pub text_hash: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DiskError {
    Missing,
    Conflict,
    Other { code: &'static str, message: String },
}

pub trait SessionIo {
    fn read(&self) -> Result<DiskSnapshot, DiskError>;
    fn hash(&self) -> Result<String, DiskError>;
    fn write(&self, content: &str, expected_hash: &str) -> Result<String, DiskError>;
    fn text_hash(&self, text: &str) -> String;
    fn preserve(&self, body: &str, report: ConflictReport) -> Result<(), DiskError>;
    fn append(&self, update: &[u8]) -> Result<(), DiskError>;
    fn sync_point(&self) -> SyncPointLookup;
    fn remember(&self, point: SyncPoint) -> Result<(), DiskError>;
}

#[derive(Debug)]
pub enum SessionError {
    Missing,
    Replica(ReplicaError),
    Change(ChangeError),
    Disk { code: &'static str, message: String },
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("the note file is missing"),
            Self::Replica(error) => write!(formatter, "{error}"),
            Self::Change(error) => write!(formatter, "{error}"),
            Self::Disk { message, .. } => formatter.write_str(message),
        }
    }
}

impl From<ReplicaError> for SessionError {
    fn from(error: ReplicaError) -> Self {
        Self::Replica(error)
    }
}

impl From<ChangeError> for SessionError {
    fn from(error: ChangeError) -> Self {
        Self::Change(error)
    }
}

fn disk_failure(error: DiskError) -> SessionError {
    match error {
        DiskError::Missing => SessionError::Missing,
        DiskError::Conflict => SessionError::Disk { code: "conflict", message: "the file changed during the write".into() },
        DiskError::Other { code, message } => SessionError::Disk { code, message },
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LoggedChange {
    pub client: String,
    pub changes: Value,
}

#[derive(Debug, PartialEq)]
pub enum Pull {
    Changes(Vec<LoggedChange>),
    Resync,
}

fn is_blank(text: &str) -> bool {
    text.chars().all(|symbol| matches!(symbol, ' ' | '\t' | '\n' | '\r'))
}

pub struct DocumentSession {
    replica: Replica,
    version: u64,
    log_start: u64,
    log: VecDeque<LoggedChange>,
    synced_content: String,
    synced_hash: String,
    dirty: bool,
}

impl DocumentSession {
    pub fn open(blobs: &[Vec<u8>], io: &impl SessionIo) -> Result<Self, SessionError> {
        let replica = Replica::restore(blobs)?;
        let snapshot = io.read().map_err(disk_failure)?;
        let mut session = Self {
            replica,
            version: 0,
            log_start: 0,
            log: VecDeque::new(),
            synced_content: snapshot.content.clone(),
            synced_hash: snapshot.hash.clone(),
            dirty: false,
        };
        let doc_text = session.replica.text();
        let verdict = resolve_sync(
            SyncInputs {
                session_base: None,
                file_hash: &snapshot.hash,
                file_text: &snapshot.content,
                doc_text: &doc_text,
            },
            || (io.sync_point(), io.text_hash(&doc_text)),
        );
        match verdict {
            SyncVerdict::InSync => session.remember_snapshot(&snapshot, io)?,
            SyncVerdict::PublishDocument => session.dirty = true,
            SyncVerdict::TakeFile => session.take_file(&snapshot, &doc_text, io)?,
            SyncVerdict::Conflict(cause) => {
                io.preserve(&doc_text, session.report(cause, &snapshot)).map_err(disk_failure)?;
                session.take_file(&snapshot, &doc_text, io)?;
            }
            SyncVerdict::Merge { base } => session.merge(&snapshot, &base, &doc_text, io)?,
        }
        Ok(session)
    }

    pub fn text(&self) -> String {
        self.replica.text()
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn state(&self) -> Vec<u8> {
        self.replica.state()
    }

    pub fn encode_position(&self, index: usize) -> Option<Vec<u8>> {
        self.replica.encode_position(index)
    }

    pub fn resolve_position(&self, encoded: &[u8]) -> Option<usize> {
        self.replica.resolve_position(encoded)
    }

    pub fn push(
        &mut self,
        client: &str,
        version: u64,
        changes: &[Value],
        io: &impl SessionIo,
    ) -> Result<bool, SessionError> {
        if version != self.version {
            return Ok(false);
        }
        let mut length = self.replica.len();
        let mut batches = Vec::with_capacity(changes.len());
        for change in changes {
            let edits = edits_from_json(change, length)?;
            length = length_after(&edits, length);
            batches.push((edits, change.clone()));
        }
        if !batches.is_empty() {
            // Make ONE atomic journal append for this entire versioned batch.
            // A failed append leaves the live replica, version and log unchanged.
            let edits: Vec<Vec<TextEdit>> =
                batches.iter().map(|(edits, _)| edits.clone()).collect();
            let update = self.replica.stage_edits(&edits)?;
            io.append(&update).map_err(disk_failure)?;
            self.replica.apply_update(&update, client)?;
            for (_, change) in batches {
                self.record(client, change);
            }
            self.dirty = true;
        }
        Ok(true)
    }

    pub fn pull(&self, version: u64) -> Pull {
        if version < self.log_start || version > self.version {
            return Pull::Resync;
        }
        let skip = (version - self.log_start) as usize;
        Pull::Changes(self.log.iter().skip(skip).cloned().collect())
    }

    pub fn replace_text(&mut self, text: &str, client: &str, io: &impl SessionIo)
        -> Result<(), SessionError>
    {
        let current = self.replica.text();
        let edits = merge_external_change(&current, text, &current).edits;
        if self.apply(&edits, client, io)? {
            self.dirty = true;
        }
        Ok(())
    }

    pub fn revert(
        &mut self,
        version_text: &str,
        previous_text: &str,
        client: &str,
        io: &impl SessionIo,
    ) -> Result<bool, SessionError> {
        let current = self.replica.text();
        let merged = merge_external_change(version_text, previous_text, &current);
        if !merged.displaced.is_empty() {
            let report = ConflictReport {
                cause: ConflictCause::Reverted,
                disk_hash: None,
                synced_hash: None,
            };
            io.preserve(&merged.displaced.join("\n"), report).map_err(disk_failure)?;
        }
        let changed = self.apply(&merged.edits, client, io)?;
        if changed {
            self.dirty = true;
        }
        Ok(changed)
    }

    pub fn reconcile(&mut self, io: &impl SessionIo) -> Result<(), SessionError> {
        if io.hash().map_err(disk_failure)? == self.synced_hash {
            return Ok(());
        }
        let snapshot = io.read().map_err(disk_failure)?;
        let doc_text = self.replica.text();
        if snapshot.content == doc_text {
            if let Err(error) = self.remember_snapshot(&snapshot, io) {
                // The file and live replica agree, but SQLite did not confirm
                // their sync point. Keep a retry pending rather than silently
                // claiming the metadata is durable.
                self.dirty = true;
                return Err(error);
            }
            self.dirty = false;
            return Ok(());
        }
        let base = self.synced_content.clone();
        self.merge(&snapshot, &base, &doc_text, io)?;
        Ok(())
    }

    pub fn write(&mut self, io: &impl SessionIo) -> Result<(), SessionError> {
        for _ in 0..2 {
            if !self.dirty {
                return Ok(());
            }
            let content = self.replica.text();
            if is_blank(&content) && !is_blank(&self.synced_content) {
                io.preserve(&self.synced_content, self.truncation_report()).map_err(disk_failure)?;
            }
            match io.write(&content, &self.synced_hash) {
                Ok(hash) => {
                    let text_hash = io.text_hash(&content);
                    // Disk bytes may be safely written while SQLite is still
                    // unavailable. Leave the previously confirmed baseline and
                    // dirty retry state untouched until remember() succeeds.
                    // On retry, a changed disk hash triggers reconcile first.
                    io.remember(SyncPoint { file_hash: hash.clone(), text_hash })
                        .map_err(disk_failure)?;
                    self.synced_content = content;
                    self.synced_hash = hash;
                    self.dirty = false;
                    return Ok(());
                }
                Err(DiskError::Conflict) => self.reconcile(io)?,
                Err(error) => return Err(disk_failure(error)),
            }
        }
        if self.dirty {
            return Err(SessionError::Disk { code: "conflict", message: "the file kept changing during the write".into() });
        }
        Ok(())
    }

    fn take_file(&mut self, snapshot: &DiskSnapshot, doc_text: &str, io: &impl SessionIo)
        -> Result<(), SessionError>
    {
        let edits = merge_external_change(doc_text, &snapshot.content, doc_text).edits;
        self.apply(&edits, DISK_CLIENT, io)?;
        self.remember_snapshot(snapshot, io)?;
        Ok(())
    }

    fn merge(
        &mut self,
        snapshot: &DiskSnapshot,
        base: &str,
        doc_text: &str,
        io: &impl SessionIo,
    ) -> Result<(), SessionError> {
        let merged = merge_external_change(base, &snapshot.content, doc_text);
        if merged.coarse {
            eprintln!("[aquilum:documents] грубое слияние: файл разошёлся с документом больше порога построчного сравнения");
        }
        // A displaced edit must have a durable protection copy BEFORE it is
        // removed from the replica by the reconciliation edits.
        if !merged.displaced.is_empty() {
            io.preserve(&merged.displaced.join("\n"), self.report(ConflictCause::Displaced, snapshot))
                .map_err(disk_failure)?;
        }
        self.apply(&merged.edits, DISK_CLIENT, io)?;
        if self.replica.text() == snapshot.content {
            if let Err(error) = self.remember_snapshot(snapshot, io) {
                // CRDT delta is in the journal, but the metadata isn't.
                self.dirty = true;
                return Err(error);
            }
            self.dirty = false;
        } else {
            self.synced_content = snapshot.content.clone();
            self.synced_hash = snapshot.hash.clone();
            self.dirty = true;
        }
        Ok(())
    }

    fn apply(&mut self, edits: &[TextEdit], client: &str, io: &impl SessionIo)
        -> Result<bool, SessionError>
    {
        if edits.is_empty() {
            return Ok(false);
        }
        let changes = edits_to_json(edits, self.replica.len());
        let update = self.replica.stage_edits(&[edits.to_vec()])?;
        // Store the delta before changing any in-memory user-visible state.
        io.append(&update).map_err(disk_failure)?;
        self.replica.apply_update(&update, client)?;
        self.record(client, changes);
        Ok(true)
    }

    fn record(&mut self, client: &str, changes: Value) {
        self.log.push_back(LoggedChange { client: client.to_owned(), changes });
        self.version += 1;
        while self.log.len() > LOG_LIMIT {
            self.log.pop_front();
            self.log_start += 1;
        }
    }

    fn remember_snapshot(&mut self, snapshot: &DiskSnapshot, io: &impl SessionIo)
        -> Result<(), SessionError>
    {
        // Never advertise a new confirmed baseline before the store has
        // accepted it. A failure must be visible to open/reconcile/write.
        io.remember(SyncPoint {
            file_hash: snapshot.hash.clone(),
            text_hash: snapshot.text_hash.clone(),
        }).map_err(disk_failure)?;
        self.synced_content = snapshot.content.clone();
        self.synced_hash = snapshot.hash.clone();
        Ok(())
    }

    fn report(&self, cause: ConflictCause, snapshot: &DiskSnapshot) -> ConflictReport {
        ConflictReport {
            cause,
            disk_hash: Some(snapshot.hash.clone()),
            synced_hash: Some(self.synced_hash.clone()),
        }
    }

    fn truncation_report(&self) -> ConflictReport {
        ConflictReport { cause: ConflictCause::Truncated, disk_hash: None, synced_hash: Some(self.synced_hash.clone()) }
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
