//! SYNC-DELTA-01b: read-only, bounded S3 head and immutable-manifest inspection.
//!
//! This is an opt-in network *observer*, not a snapshot consistency or sync
//! implementation. A valid head only identifies a candidate immutable manifest.
//! A matching head does not prove that local files are unchanged, and digest
//! matching does not authenticate the writer or validate the manifest schema.
//!
//! No mutation, listing, baseline advancement, CAS publish, or deletion occurs.

use super::s3::{S3ContractError, S3HeadPointerV1, S3Keyspace};
use super::s3_transport::{
    AwsS3ProbeClient, S3BoundedObject, S3TransportError,
};

const MAX_HEAD_BYTES: u64 = 4 * 1024;
const MAX_MANIFEST_PROBE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3HeadReadError {
    Transport(S3TransportError),
    InvalidPointer(S3ContractError),
    MalformedReadback,
    ManifestHashMismatch,
    /// The current pointer changed while reading the referenced manifest.
    HeadChangedDuringRead,
}

impl From<S3TransportError> for S3HeadReadError {
    fn from(error: S3TransportError) -> Self {
        Self::Transport(error)
    }
}

impl From<S3ContractError> for S3HeadReadError {
    fn from(error: S3ContractError) -> Self {
        Self::InvalidPointer(error)
    }
}

/// An observed, vault-scoped head with provider ETag.
/// This is NOT a verified complete remote snapshot or authority to modify files.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3ObservedHead {
    pointer: S3HeadPointerV1,
    etag: String,
    original_bytes: Vec<u8>,
}

impl S3ObservedHead {
    pub fn pointer(&self) -> &S3HeadPointerV1 {
        &self.pointer
    }

    /// Opaque CAS validator, not a hash of the manifest or note contents.
    pub fn etag(&self) -> &str {
        &self.etag
    }
}

/// A bounded immutable object's bytes whose BLAKE3 matches the observed head.
///
/// Only the *content digest* has been checked; no canonical manifest schema,
/// complete revision inventory, chain ancestry, writer authenticity or
/// filesystem authority has been established by this return value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3DigestMatchedManifest {
    pub head: S3ObservedHead,
    pub bytes: Vec<u8>,
}

/// Read only the small current-head object; no S3 listing or manifest download.
///
/// Callers may compare the pointer/ETag against a *previously verified and
/// provider-profile-bound* local checkpoint. Such a comparison cannot by itself
/// mark a sync complete: local pending edits and the File Gate still matter.
pub async fn observe_s3_current_head(
    client: &AwsS3ProbeClient,
    keyspace: &S3Keyspace,
) -> Result<S3ObservedHead, S3HeadReadError> {
    observe_with(client, keyspace).await
}

/// Fetch the manifest identified by a previously observed root, verify its
/// BLAKE3, then re-read that root to catch concurrent S3 CAS changes.
///
/// Does not parse the manifest as a complete inventory and does not advance the
/// SQLite journal baseline. Reading a missing root or manifest fails closed.
pub async fn read_s3_digest_matched_manifest(
    client: &AwsS3ProbeClient,
    keyspace: &S3Keyspace,
    observed: S3ObservedHead,
) -> Result<S3DigestMatchedManifest, S3HeadReadError> {
    manifest_with(client, keyspace, observed).await
}

fn check_readback(
    object: &S3BoundedObject,
    max_bytes: u64,
) -> Result<(), S3HeadReadError> {
    if object.object.etag.is_empty()
        || object.bytes.len() as u64 > max_bytes
        || object.object.byte_length != object.bytes.len() as u64
        || blake3::hash(&object.bytes).to_hex().as_str() != object.blake3
    {
        return Err(S3HeadReadError::MalformedReadback);
    }
    Ok(())
}

#[allow(async_fn_in_trait)]
trait BoundedReader {
    async fn read_bounded(
        &self,
        key: &str,
        max_bytes: u64,
    ) -> Result<S3BoundedObject, S3TransportError>;
}

impl BoundedReader for AwsS3ProbeClient {
    async fn read_bounded(
        &self,
        key: &str,
        max_bytes: u64,
    ) -> Result<S3BoundedObject, S3TransportError> {
        AwsS3ProbeClient::get_bounded(self, key, max_bytes).await
    }
}

async fn observe_with<R: BoundedReader>(
    reader: &R,
    keyspace: &S3Keyspace,
) -> Result<S3ObservedHead, S3HeadReadError> {
    let response = reader
        .read_bounded(&keyspace.current_head_key(), MAX_HEAD_BYTES)
        .await?;
    check_readback(&response, MAX_HEAD_BYTES)?;
    let pointer = S3HeadPointerV1::decode(&response.bytes, keyspace)?;
    Ok(S3ObservedHead {
        pointer,
        etag: response.object.etag,
        original_bytes: response.bytes,
    })
}

async fn manifest_with<R: BoundedReader>(
    reader: &R,
    keyspace: &S3Keyspace,
    observed: S3ObservedHead,
) -> Result<S3DigestMatchedManifest, S3HeadReadError> {
    // Also revalidates the pointer against the expected vault namespace.
    let manifest_key = observed.pointer.immutable_manifest_key(keyspace)?;
    let response = reader
        .read_bounded(&manifest_key, MAX_MANIFEST_PROBE_BYTES)
        .await?;
    check_readback(&response, MAX_MANIFEST_PROBE_BYTES)?;
    let digest = blake3::hash(&response.bytes).to_hex().to_string();
    if digest != observed.pointer.manifest_blake3() {
        return Err(S3HeadReadError::ManifestHashMismatch);
    }

    let latest = observe_with(reader, keyspace).await?;
    if latest.pointer != observed.pointer
        || latest.etag != observed.etag
        || latest.original_bytes != observed.original_bytes
    {
        return Err(S3HeadReadError::HeadChangedDuringRead);
    }

    Ok(S3DigestMatchedManifest {
        head: observed,
        bytes: response.bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{OperationId, VaultId};
    use super::super::s3_transport::S3ObservedObject;
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::Mutex;

    #[derive(Default)]
    struct ScriptedReader {
        responses: Mutex<BTreeMap<String, VecDeque<Result<S3BoundedObject, S3TransportError>>>>,
        calls: Mutex<Vec<String>>,
    }

    impl ScriptedReader {
        fn set(
            &self,
            key: String,
            replies: Vec<Result<S3BoundedObject, S3TransportError>>,
        ) {
            assert!(!replies.is_empty());
            self.responses
                .lock()
                .unwrap()
                .insert(key, VecDeque::from(replies));
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl BoundedReader for ScriptedReader {
        async fn read_bounded(
            &self,
            key: &str,
            max_bytes: u64,
        ) -> Result<S3BoundedObject, S3TransportError> {
            self.calls.lock().unwrap().push(key.to_owned());
            let value = {
                let mut state = self.responses.lock().unwrap();
                let queue = state.get_mut(key).ok_or(S3TransportError::NotFound)?;
                if queue.len() > 1 {
                    queue.pop_front().unwrap()
                } else {
                    queue.front().cloned().ok_or(S3TransportError::NotFound)?
                }
            }?;
            if value.bytes.len() as u64 > max_bytes {
                return Err(S3TransportError::ObjectTooLarge);
            }
            Ok(value)
        }
    }

    fn bounded(bytes: &[u8]) -> S3BoundedObject {
        let digest = blake3::hash(bytes).to_hex().to_string();
        S3BoundedObject {
            object: S3ObservedObject {
                etag: format!("\"{digest}\""),
                byte_length: bytes.len() as u64,
            },
            bytes: bytes.to_vec(),
            blake3: digest,
        }
    }

    fn fixture() -> (S3Keyspace, S3HeadPointerV1, Vec<u8>, ScriptedReader) {
        let keyspace = S3Keyspace::new("akula/sync", VaultId::new()).unwrap();
        let manifest = b"opaque immutable delta manifest fixture".to_vec();
        let id = OperationId::new();
        let pointer = S3HeadPointerV1::new(
            &keyspace,
            1,
            id,
            &blake3::hash(&manifest).to_hex().to_string(),
            None,
        )
        .unwrap();
        let root_bytes = pointer.encode(&keyspace).unwrap();
        let reader = ScriptedReader::default();
        reader.set(
            keyspace.current_head_key(),
            vec![Ok(bounded(&root_bytes))],
        );
        reader.set(
            keyspace.immutable_manifest_key(id),
            vec![Ok(bounded(&manifest))],
        );
        (keyspace, pointer, manifest, reader)
    }

    #[tokio::test]
    async fn unchanged_checkpoint_can_be_compared_after_one_root_read() {
        let (keyspace, pointer, _, reader) = fixture();
        let head = observe_with(&reader, &keyspace).await.unwrap();
        assert_eq!(head.pointer(), &pointer);
        assert!(!head.etag().is_empty());
        assert_eq!(reader.calls(), vec![keyspace.current_head_key()]);
        // Caller still has to check its verified local checkpoint and pending edits.
    }

    #[tokio::test]
    async fn new_head_reads_only_one_manifest_and_rechecks_root() {
        let (keyspace, pointer, manifest, reader) = fixture();
        let head = observe_with(&reader, &keyspace).await.unwrap();
        let checked = manifest_with(&reader, &keyspace, head).await.unwrap();
        assert_eq!(checked.head.pointer(), &pointer);
        assert_eq!(checked.bytes, manifest);
        assert_eq!(
            reader.calls(),
            vec![
                keyspace.current_head_key(),
                keyspace.immutable_manifest_key(pointer.manifest_id()),
                keyspace.current_head_key()
            ]
        );
    }

    #[tokio::test]
    async fn wrong_manifest_digest_and_missing_manifest_fail_closed() {
        let (keyspace, pointer, _, reader) = fixture();
        reader.set(
            keyspace.immutable_manifest_key(pointer.manifest_id()),
            vec![Ok(bounded(b"tampered payload"))],
        );
        let head = observe_with(&reader, &keyspace).await.unwrap();
        assert_eq!(
            manifest_with(&reader, &keyspace, head).await,
            Err(S3HeadReadError::ManifestHashMismatch)
        );

        let (keyspace, pointer, _, reader) = fixture();
        reader.set(
            keyspace.immutable_manifest_key(pointer.manifest_id()),
            vec![Err(S3TransportError::NotFound)],
        );
        let head = observe_with(&reader, &keyspace).await.unwrap();
        assert_eq!(
            manifest_with(&reader, &keyspace, head).await,
            Err(S3HeadReadError::Transport(S3TransportError::NotFound))
        );
    }

    #[tokio::test]
    async fn concurrent_cas_head_change_rejects_stale_manifest() {
        let (keyspace, pointer, _, reader) = fixture();
        let initial = pointer.encode(&keyspace).unwrap();
        let changed = S3HeadPointerV1::new(
            &keyspace,
            2,
            OperationId::new(),
            &blake3::hash(b"next").to_hex().to_string(),
            Some(pointer.manifest_id()),
        )
        .unwrap()
        .encode(&keyspace)
        .unwrap();
        reader.set(
            keyspace.current_head_key(),
            vec![Ok(bounded(&initial)), Ok(bounded(&changed))],
        );
        let observed = observe_with(&reader, &keyspace).await.unwrap();
        assert_eq!(
            manifest_with(&reader, &keyspace, observed).await,
            Err(S3HeadReadError::HeadChangedDuringRead)
        );
    }

    #[tokio::test]
    async fn foreign_vault_corrupt_head_and_not_found_are_not_empty_snapshots() {
        let (keyspace, pointer, _, reader) = fixture();
        let observed = observe_with(&reader, &keyspace).await.unwrap();
        let other = S3Keyspace::new("akula/sync", VaultId::new()).unwrap();
        assert_eq!(
            manifest_with(&reader, &other, observed).await,
            Err(S3HeadReadError::InvalidPointer(S3ContractError::ForeignVault))
        );

        let (keyspace, _, _, reader) = fixture();
        reader.set(
            keyspace.current_head_key(),
            vec![Ok(bounded(b"invalid json"))],
        );
        assert_eq!(
            observe_with(&reader, &keyspace).await,
            Err(S3HeadReadError::InvalidPointer(S3ContractError::InvalidRoot))
        );

        let (keyspace, _, _, reader) = fixture();
        reader.set(
            keyspace.current_head_key(),
            vec![Err(S3TransportError::NotFound)],
        );
        assert_eq!(
            observe_with(&reader, &keyspace).await,
            Err(S3HeadReadError::Transport(S3TransportError::NotFound))
        );
    }

    #[tokio::test]
    async fn oversized_or_internally_inconsistent_reads_are_rejected() {
        let (keyspace, _, _, reader) = fixture();
        reader.set(
            keyspace.current_head_key(),
            vec![Ok(bounded(&vec![0_u8; 4097]))],
        );
        assert_eq!(
            observe_with(&reader, &keyspace).await,
            Err(S3HeadReadError::Transport(S3TransportError::ObjectTooLarge))
        );

        let (keyspace, _, _, reader) = fixture();
        let mut fake = bounded(b"{}");
        fake.object.byte_length += 1;
        reader.set(keyspace.current_head_key(), vec![Ok(fake)]);
        assert_eq!(
            observe_with(&reader, &keyspace).await,
            Err(S3HeadReadError::MalformedReadback)
        );
    }

    #[tokio::test]
    async fn changed_etag_with_identical_root_bytes_is_not_silently_accepted() {
        let (keyspace, pointer, _, reader) = fixture();
        let first = bounded(&pointer.encode(&keyspace).unwrap());
        let mut later = first.clone();
        later.object.etag = "\"different-valid-opaque-etag\"".into();
        reader.set(
            keyspace.current_head_key(),
            vec![Ok(first), Ok(later)],
        );
        let observed = observe_with(&reader, &keyspace).await.unwrap();
        assert_eq!(
            manifest_with(&reader, &keyspace, observed).await,
            Err(S3HeadReadError::HeadChangedDuringRead)
        );
    }
}
