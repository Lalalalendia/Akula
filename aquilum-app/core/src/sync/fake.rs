//! Deterministic, in-memory RemoteStore for PR 00 contract tests and later
//! two-client fault injection. This is never a production cloud adapter.

use super::provider::{
    ProviderCapabilities, RemoteObject, RemotePage, RemotePrecondition, RemoteStore,
    RemoteStoreError, RemoteWriteReceipt,
};
use std::collections::BTreeMap;

struct StoredObject {
    revision: String,
    bytes: Vec<u8>,
}

#[derive(Default)]
pub struct FakeRemoteStore {
    objects: BTreeMap<String, StoredObject>,
    generation: u64,
    lose_next_write_ack: bool,
    incomplete_listing: bool,
}

impl FakeRemoteStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Simulates a successful remote write whose HTTP response was lost.
    pub fn lose_next_write_ack(&mut self) {
        self.lose_next_write_ack = true;
    }

    /// Simulates a provider that cannot prove its last listing page is complete.
    pub fn set_incomplete_listing(&mut self, incomplete: bool) {
        self.incomplete_listing = incomplete;
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    fn authorize(
        existing: Option<&StoredObject>,
        precondition: &RemotePrecondition,
    ) -> Result<(), RemoteStoreError> {
        let matches = match (existing, precondition) {
            (None, RemotePrecondition::Absent) => true,
            (Some(object), RemotePrecondition::Matches(expected)) => {
                &object.revision == expected
            }
            _ => false,
        };
        if matches {
            Ok(())
        } else {
            Err(RemoteStoreError::PreconditionFailed)
        }
    }

    fn object(id: &str, stored: &StoredObject) -> RemoteObject {
        RemoteObject {
            id: id.to_owned(),
            revision: stored.revision.clone(),
            byte_length: stored.bytes.len() as u64,
            content_hash: Some(blake3::hash(&stored.bytes).to_hex().to_string()),
        }
    }
}

impl RemoteStore for FakeRemoteStore {
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            conditional_create: true,
            conditional_replace: true,
            conditional_delete: true,
            snapshot_consistency: true,
            version_history: false,
            delta_feed: false,
        }
    }

    fn list_page(
        &self,
        continuation: Option<&str>,
        page_limit: usize,
    ) -> Result<RemotePage, RemoteStoreError> {
        if page_limit == 0 {
            return Err(RemoteStoreError::InvalidCursor);
        }
        let offset = if let Some(continuation) = continuation {
            let (generation, position) = continuation
                .split_once(':')
                .ok_or(RemoteStoreError::InvalidCursor)?;
            let generation: u64 = generation
                .parse()
                .map_err(|_| RemoteStoreError::InvalidCursor)?;
            if generation != self.generation {
                return Err(RemoteStoreError::ListingChanged);
            }
            position
                .parse::<usize>()
                .map_err(|_| RemoteStoreError::InvalidCursor)?
        } else {
            0
        };
        if offset > self.objects.len() {
            return Err(RemoteStoreError::InvalidCursor);
        }
        let end = offset.saturating_add(page_limit).min(self.objects.len());
        if end == self.objects.len() && self.incomplete_listing {
            return Err(RemoteStoreError::IncompleteListing);
        }
        let objects = self
            .objects
            .iter()
            .skip(offset)
            .take(end - offset)
            .map(|(id, record)| Self::object(id, record))
            .collect();
        let complete = end == self.objects.len();
        Ok(RemotePage {
            objects,
            next_cursor: (!complete).then(|| format!("{}:{end}", self.generation)),
            complete,
            snapshot_id: self.generation.to_string(),
        })
    }

    fn read(&self, object_id: &str) -> Result<Vec<u8>, RemoteStoreError> {
        self.objects
            .get(object_id)
            .map(|record| record.bytes.clone())
            .ok_or(RemoteStoreError::NotFound)
    }

    fn read_bounded(&self, object_id: &str, max_bytes: u64)
        -> Result<Vec<u8>, RemoteStoreError>
    {
        let record = self.objects.get(object_id).ok_or(RemoteStoreError::NotFound)?;
        if record.bytes.len() as u64 > max_bytes {
            return Err(RemoteStoreError::ObjectTooLarge);
        }
        Ok(record.bytes.clone())
    }

    fn head(&self, object_id: &str) -> Result<RemoteObject, RemoteStoreError> {
        self.objects
            .get(object_id)
            .map(|record| Self::object(object_id, record))
            .ok_or(RemoteStoreError::NotFound)
    }

    fn put(
        &mut self,
        object_id: &str,
        bytes: &[u8],
        precondition: &RemotePrecondition,
    ) -> Result<RemoteWriteReceipt, RemoteStoreError> {
        if object_id.is_empty() {
            return Err(RemoteStoreError::InvalidCursor);
        }
        Self::authorize(self.objects.get(object_id), precondition)?;
        self.generation += 1;
        let revision = self.generation.to_string();
        self.objects.insert(
            object_id.to_owned(),
            StoredObject {
                revision: revision.clone(),
                bytes: bytes.to_owned(),
            },
        );
        if std::mem::take(&mut self.lose_next_write_ack) {
            return Err(RemoteStoreError::CommitUncertain);
        }
        Ok(RemoteWriteReceipt { revision })
    }

    fn delete(
        &mut self,
        object_id: &str,
        precondition: &RemotePrecondition,
    ) -> Result<(), RemoteStoreError> {
        Self::authorize(self.objects.get(object_id), precondition)?;
        if self.objects.remove(object_id).is_none() {
            return Err(RemoteStoreError::NotFound);
        }
        self.generation += 1;
        Ok(())
    }
}


/// Counts successful mock RemoteStore payload bytes, not on-wire IP/QUIC bytes.
/// Kept test-only: these numbers must never be presented as real network traffic.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PayloadIoCounts {
    pub head_calls: u64,
    pub get_calls: u64,
    pub get_bytes: u64,
    pub put_calls: u64,
    pub put_bytes: u64,
}

/// An independent, test-only byte meter around the existing fake provider.
/// A failed precondition does not count as transferred payload. An uncertain
/// write ACK does, since the fake provider has already accepted the bytes.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct MeteredFakeRemoteStore {
    inner: FakeRemoteStore,
    counts: std::cell::Cell<PayloadIoCounts>,
}

#[cfg(test)]
impl MeteredFakeRemoteStore {
    pub fn counts(&self) -> PayloadIoCounts {
        self.counts.get()
    }

    pub fn reset_counts(&self) {
        self.counts.set(PayloadIoCounts::default());
    }
}

#[cfg(test)]
impl RemoteStore for MeteredFakeRemoteStore {
    fn capabilities(&self) -> ProviderCapabilities {
        self.inner.capabilities()
    }

    fn list_page(
        &self,
        continuation: Option<&str>,
        page_limit: usize,
    ) -> Result<RemotePage, RemoteStoreError> {
        self.inner.list_page(continuation, page_limit)
    }

    fn read(&self, object_id: &str) -> Result<Vec<u8>, RemoteStoreError> {
        let bytes = self.inner.read(object_id)?;
        let mut counts = self.counts.get();
        counts.get_calls += 1;
        counts.get_bytes += bytes.len() as u64;
        self.counts.set(counts);
        Ok(bytes)
    }

    fn read_bounded(
        &self,
        object_id: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, RemoteStoreError> {
        let bytes = self.inner.read_bounded(object_id, max_bytes)?;
        let mut counts = self.counts.get();
        counts.get_calls += 1;
        counts.get_bytes += bytes.len() as u64;
        self.counts.set(counts);
        Ok(bytes)
    }

    fn head(&self, object_id: &str) -> Result<RemoteObject, RemoteStoreError> {
        let mut counts = self.counts.get();
        counts.head_calls += 1;
        self.counts.set(counts);
        self.inner.head(object_id)
    }

    fn put(
        &mut self,
        object_id: &str,
        bytes: &[u8],
        precondition: &RemotePrecondition,
    ) -> Result<RemoteWriteReceipt, RemoteStoreError> {
        let result = self.inner.put(object_id, bytes, precondition);
        if matches!(&result, Ok(_) | Err(RemoteStoreError::CommitUncertain)) {
            let mut counts = self.counts.get();
            counts.put_calls += 1;
            counts.put_bytes += bytes.len() as u64;
            self.counts.set(counts);
        }
        result
    }

    fn delete(
        &mut self,
        object_id: &str,
        precondition: &RemotePrecondition,
    ) -> Result<(), RemoteStoreError> {
        self.inner.delete(object_id, precondition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_versions_enforce_conditional_writes() {
        let mut remote = FakeRemoteStore::new();
        let first = remote.put("vault/a.md", b"old", &RemotePrecondition::Absent).unwrap();
        assert_eq!(remote.read("vault/a.md").unwrap(), b"old");
        let second = remote.put(
            "vault/a.md",
            b"new",
            &RemotePrecondition::Matches(first.revision.clone()),
        ).unwrap();
        assert_ne!(first.revision, second.revision);
        assert_eq!(
            remote.put("vault/a.md", b"lost", &RemotePrecondition::Matches(first.revision)),
            Err(RemoteStoreError::PreconditionFailed)
        );
        assert_eq!(remote.read("vault/a.md").unwrap(), b"new");
        assert!(remote.capabilities().supports_guarded_mutation());
    }

    #[test]
    fn listing_requires_all_pages_in_one_stable_snapshot() {
        let mut remote = FakeRemoteStore::new();
        for name in ["a", "b", "c"] {
            remote.put(name, name.as_bytes(), &RemotePrecondition::Absent).unwrap();
        }
        let first = remote.list_page(None, 2).unwrap();
        assert!(!first.is_final_verified_page());
        assert_eq!(first.objects.len(), 2);
        let last = remote.list_page(first.next_cursor.as_deref(), 2).unwrap();
        assert_eq!(last.objects.len(), 1);
        assert!(last.is_final_verified_page());
        assert_eq!(first.snapshot_id, last.snapshot_id);
        remote.put("d", b"new", &RemotePrecondition::Absent).unwrap();
        assert_eq!(
            remote.list_page(first.next_cursor.as_deref(), 2),
            Err(RemoteStoreError::ListingChanged)
        );
    }

    #[test]
    fn ambiguous_commit_must_be_resolved_by_readback() {
        let mut remote = FakeRemoteStore::new();
        remote.lose_next_write_ack();
        assert_eq!(
            remote.put("a", b"durable", &RemotePrecondition::Absent),
            Err(RemoteStoreError::CommitUncertain)
        );
        assert_eq!(remote.read("a").unwrap(), b"durable");
        assert_eq!(
            remote.put("a", b"duplicate", &RemotePrecondition::Absent),
            Err(RemoteStoreError::PreconditionFailed)
        );
        assert_eq!(remote.head("a").unwrap().content_hash.unwrap(),
            blake3::hash(b"durable").to_hex().to_string());
    }

    #[test]
    fn a_partial_provider_listing_is_never_an_empty_vault() {
        let mut remote = FakeRemoteStore::new();
        remote.set_incomplete_listing(true);
        assert_eq!(remote.list_page(None, 100),
            Err(RemoteStoreError::IncompleteListing));
    }

    #[test]
    fn stale_delete_cannot_remove_a_new_remote_revision() {
        let mut remote = FakeRemoteStore::new();
        let first = remote.put("a", b"r1", &RemotePrecondition::Absent).unwrap();
        let second = remote.put("a", b"r2",
            &RemotePrecondition::Matches(first.revision.clone())).unwrap();
        assert_eq!(
            remote.delete("a", &RemotePrecondition::Matches(first.revision)),
            Err(RemoteStoreError::PreconditionFailed)
        );
        assert_eq!(remote.read("a").unwrap(), b"r2");
        remote.delete("a", &RemotePrecondition::Matches(second.revision)).unwrap();
        assert_eq!(remote.read("a"), Err(RemoteStoreError::NotFound));
    }
}
