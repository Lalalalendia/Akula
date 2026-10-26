//! SYNC-05b: isolated, opt-in AWS S3 transport and conditional-write probe.
//!
//! This is NOT a RemoteStore implementation: ordinary ListObjectsV2 cannot
//! satisfy the manifest-backed, stable snapshot contract in provider.rs.
//! There is no scheduler, credential persistence, local file operation, remote
//! deletion, plaintext mirror or multipart upload in this slice.
//!
//! Run these async calls in a dedicated Core worker; never from native UI.

use super::s3::S3Keyspace;
use super::protection_store::MAX_PROTECTED_ORIGINAL_BYTES;

mod multipart;
mod explicit_credentials;
pub(crate) mod pair_upload;
pub(crate) mod pair_receive;
pub use explicit_credentials::S3PairAccess;
mod manifest;
mod principal;
pub use principal::{S3AwsAccountProofError, S3AwsVerifiedAccountScope};
mod v2_readonly;
pub use v2_readonly::{S3PinnedV2ReadError, S3PinnedV2ReadPreview};
use super::s3_pack_probe as pack_probe;
// Pure bounded pack encoder for future host-admitted sender; no S3 writer.
mod coord_pack_encode;
// SYNC-COORD-01 stays test-only: no production S3 format or file writes.
#[cfg(test)]
mod coord_probe;
// Heavy full-DAG benchmark is ignored unless explicitly run with MinIO.
#[cfg(test)]
mod scale_probe;
// Isolated real MinIO scale A/B for anchored, read-only V2 pack discovery.
#[cfg(test)]
mod coord_scale_probe;
mod replan;
mod core_stage;
mod guarded_update_plan;
mod existing_content_stage;
mod additive_receive;
pub use replan::{plan_verified_s3_revision_replay, S3CasReplanPreview, S3ReplanError};
mod snapshot_store;
mod verified_cache_disk;
pub use verified_cache_disk::{S3VerifiedHintsError, S3VerifiedHintsJournal};
mod coord_sidecar;
pub use coord_sidecar::{S3CoordSidecarReadError, S3CoordSidecarPreview};
mod coord_chain;
pub use coord_chain::{S3CoordOfflineChainError, S3CoordOfflineChainPreview};
mod coord_selective_packs;
pub use coord_selective_packs::S3CoordSelectivePacksPreview;
mod coord_locator;
// V2 immutable packs/letter/locator prepublication; never mutable CAS.
mod coord_immutable_publish;
mod selective_snapshot;
mod offline_retention;
pub use selective_snapshot::{S3SelectiveReadError, S3SelectiveRevisionPreview};
pub use snapshot_store::{
    S3VerifiedRevisionMetadataStore, MAX_S3_REVISION_METADATA_CACHE_BYTES,
};
pub use manifest::{
    S3HeadAttemptPhase, S3HeadError, S3HeadJournal, S3HeadPublication, S3VerifiedHead,
    S3PinnedRevisionPage, S3PinnedRevisionSnapshot,
};
pub use multipart::{
    S3MultipartError, S3MultipartIntent, S3MultipartJournal, S3MultipartOutcome,
    S3MultipartPhase, S3MultipartStatus, S3StreamDigest,
};
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::primitives::ByteStream;
use std::time::Duration;

#[cfg(test)]
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};
/// Test-only S3 method-attempt instrumentation. These count Akula SDK
/// HEAD/GET attempts and fully read payload bytes, not TCP bytes, retries
/// inside the AWS SDK, TLS frames or server-side network traffic.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct S3TestIoCounts {
    pub head: u64,
    pub get: u64,
    pub payload_bytes: u64,
}

#[cfg(test)]
impl S3TestIoCounts {
    pub(super) fn since(self, before: Self) -> Self {
        Self {
            head: self.head.checked_sub(before.head).unwrap(),
            get: self.get.checked_sub(before.get).unwrap(),
            payload_bytes: self.payload_bytes.checked_sub(before.payload_bytes).unwrap(),
        }
    }
}

#[cfg(test)]
#[derive(Default)]
struct S3TestIoCounters {
    head: AtomicU64,
    get: AtomicU64,
    payload_bytes: AtomicU64,
}

const MAX_PROBE_BYTES: u64 = 1024 * 1024;
const MAX_CORE_STAGE_BLOB_BYTES: u64 = MAX_PROTECTED_ORIGINAL_BYTES as u64;
const MAX_S3_PAGE_SIZE: i32 = 1000;

// Shared with offline S3 invite V1. S3 transport retains the exact same
// validator and errors, but it no longer owns those portable types.
pub use super::s3_pair_connection::{S3ConnectionProfile, S3TransportError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3ObservedObject {
    pub etag: String,
    pub byte_length: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3BoundedObject {
    pub object: S3ObservedObject,
    pub bytes: Vec<u8>,
    pub blake3: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3DiagnosticPage {
    pub keys: Vec<String>,
    pub next_cursor: Option<String>,
    pub complete: bool,
}

/// Transport-level condition. The server MUST enforce this atomically.
/// An unguarded GET/HEAD followed by PUT is never acceptable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3PutCondition {
    IfAbsent,
    IfMatch(String),
}

impl S3PutCondition {
    fn validate(&self) -> Result<(), S3TransportError> {
        match self {
            Self::IfAbsent => Ok(()),
            Self::IfMatch(etag) if etag.len() <= 256 && !etag.is_empty()
                && !etag.bytes().any(|ch| ch.is_ascii_control()) => Ok(()),
            _ => Err(S3TransportError::InvalidProfile),
        }
    }
}

/// An authenticated SDK client scoped to one Akula vault prefix.
/// Credentials come from the caller-controlled AWS provider chain, not from
/// the vault, local journal, ordinary settings JSON, logs or object names.
#[derive(Clone)]
pub struct AwsS3ProbeClient {
    client: aws_sdk_s3::Client,
    bucket: String,
    keyspace: S3Keyspace,
    /// Non-secret endpoint/bucket/region identity used to bind durable anchors.
    profile: S3ConnectionProfile,
    /// Same resolved SDK provider as S3, but ONLY available for real AWS endpoints.
    /// No STS session or credential bytes are persisted.
    sts_client: Option<aws_sdk_sts::Client>,
    #[cfg(test)]
    test_io: Arc<S3TestIoCounters>,
}

impl AwsS3ProbeClient {
    pub async fn connect(
        profile: S3ConnectionProfile,
        keyspace: S3Keyspace,
    ) -> Result<Self, S3TransportError> {
        profile.validate()?;
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(profile.region.clone()))
            .timeout_config(
                aws_config::timeout::TimeoutConfig::builder()
                    .connect_timeout(Duration::from_secs(5))
                    .operation_timeout(Duration::from_secs(20))
                    .build(),
            )
            .load()
            .await;
        let mut config = aws_sdk_s3::config::Builder::from(&shared)
            .force_path_style(profile.force_path_style);
        if let Some(endpoint) = profile.endpoint.clone() {
            config = config.endpoint_url(endpoint);
        } else {
            // AWS_ENDPOINT_URL[_S3] and shared ~/.aws/config can otherwise
            // silently redirect signed requests around our HTTPS/loopback
            // validation. No custom endpoint means *AWS regional S3*.
            config.set_endpoint_url(None);
        }
        let sts_client = if profile.endpoint.is_none() {
            let mut sts_config = aws_sdk_sts::config::Builder::from(&shared);
            // A shared AWS_ENDPOINT_URL[_STS] must never redirect identity
            // checks to a caller-controlled endpoint returning forged IDs.
            sts_config.set_endpoint_url(None);
            Some(aws_sdk_sts::Client::from_conf(sts_config.build()))
        } else {
            None
        };
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(config.build()),
            bucket: profile.bucket.clone(),
            keyspace,
            profile,
            sts_client,
            #[cfg(test)]
            test_io: Arc::new(S3TestIoCounters::default()),
        })
    }

    #[cfg(test)]
    pub(super) fn test_io_snapshot(&self) -> S3TestIoCounts {
        S3TestIoCounts {
            head: self.test_io.head.load(Ordering::Relaxed),
            get: self.test_io.get.load(Ordering::Relaxed),
            payload_bytes: self.test_io.payload_bytes.load(Ordering::Relaxed),
        }
    }

    pub(super) fn bound_profile(&self) -> &S3ConnectionProfile {
        &self.profile
    }

    pub(super) fn bound_keyspace(&self) -> &S3Keyspace {
        &self.keyspace
    }

    fn checked_key(&self, key: &str) -> Result<(), S3TransportError> {
        // All operations, including read-only probes, are restricted to the
        // user's configured Akula object namespace.
        if !self.keyspace.contains_key(key) {
            return Err(S3TransportError::OutsideVaultNamespace);
        }
        Ok(())
    }

    pub async fn head(&self, key: &str) -> Result<S3ObservedObject, S3TransportError> {
        self.checked_key(key)?;
        #[cfg(test)]
        self.test_io.head.fetch_add(1, Ordering::Relaxed);
        let response = self.client.head_object()
            .bucket(&self.bucket)
            .key(key)
            .send().await
            .map_err(|error| classify(error.as_service_error().and_then(|e| e.code())))?;
        let length = response.content_length()
            .ok_or(S3TransportError::MalformedResponse)?;
        let etag = response.e_tag()
            .filter(|v| !v.is_empty())
            .ok_or(S3TransportError::MalformedResponse)?;
        if length < 0 {
            return Err(S3TransportError::MalformedResponse);
        }
        Ok(S3ObservedObject { etag: etag.to_owned(), byte_length: length as u64 })
    }

    /// Legacy stage-05b pointer, metadata and diagnostic reads keep their
    /// original 1 MiB ceiling. This method never accepts large blob content.
    pub async fn get_bounded(
        &self, key: &str, max_bytes: u64,
    ) -> Result<S3BoundedObject, S3TransportError> {
        self.get_bounded_with_ceiling(key, max_bytes, MAX_PROBE_BYTES).await
    }

    /// Only immutable content-addressed blobs can use the larger capped read.
    /// HEAD / If-Match GET / HEAD and BLAKE3 must all agree before any bytes
    /// are handed to the Core cache; S3 ETag is NEVER the content digest.
    /// The hard cap equals the existing native additive File Gate limit.
    pub(crate) async fn get_immutable_blob_for_core_stage(
        &self, expected_blake3: &str, max_bytes: u64,
    ) -> Result<S3BoundedObject, S3TransportError> {
        if max_bytes == 0 || max_bytes > MAX_CORE_STAGE_BLOB_BYTES {
            return Err(S3TransportError::ObjectTooLarge);
        }
        let key = self.keyspace.immutable_blob_key(expected_blake3)
            .map_err(|_| S3TransportError::InvalidProfile)?;
        let object = self.get_bounded_with_ceiling(
            &key, max_bytes, MAX_CORE_STAGE_BLOB_BYTES,
        ).await?;
        if object.blake3 != expected_blake3 {
            return Err(S3TransportError::ChangedDuringRead);
        }
        Ok(object)
    }

    /// Stream in SDK chunks and keep a fixed caller-checked in-memory ceiling.
    /// The Content-Length is checked BEFORE allocation, each received chunk is
    /// checked independently, and final size/ETag is rechecked after download.
    /// No S3 provider response is trusted as proof of local file publication.
    async fn get_bounded_with_ceiling(
        &self, key: &str, max_bytes: u64, ceiling: u64,
    ) -> Result<S3BoundedObject, S3TransportError> {
        self.checked_key(key)?;
        if max_bytes > ceiling {
            return Err(S3TransportError::ObjectTooLarge);
        }
        let before = self.head(key).await?;
        if before.byte_length > max_bytes {
            return Err(S3TransportError::ObjectTooLarge);
        }
        #[cfg(test)]
        self.test_io.get.fetch_add(1, Ordering::Relaxed);
        let response = self.client.get_object()
            .bucket(&self.bucket)
            .key(key)
            .if_match(&before.etag)
            .send().await
            .map_err(|error| classify(error.as_service_error().and_then(|e| e.code())))?;
        let mut stream = response.body;
        let mut bytes = Vec::with_capacity(before.byte_length as usize);
        while let Some(chunk) = stream.try_next().await
            .map_err(|_| S3TransportError::Unavailable)?
        {
            if chunk.len() as u64 > max_bytes.saturating_sub(bytes.len() as u64) {
                return Err(S3TransportError::ObjectTooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let after = self.head(key).await?;
        if before != after || bytes.len() as u64 != after.byte_length {
            return Err(S3TransportError::ChangedDuringRead);
        }
        #[cfg(test)]
        self.test_io.payload_bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        let blake3 = blake3::hash(&bytes).to_hex().to_string();
        Ok(S3BoundedObject { object: after, bytes, blake3 })
    }

    /// Only small, test/pointer-sized guarded PUTs. Large immutable binary
    /// blobs and their multipart state machine remain stage 05c.
    pub async fn put_conditionally(
        &self,
        key: &str,
        bytes: &[u8],
        condition: S3PutCondition,
    ) -> Result<S3ObservedObject, S3TransportError> {
        self.checked_key(key)?;
        condition.validate()?;
        if bytes.len() as u64 > MAX_PROBE_BYTES {
            return Err(S3TransportError::ObjectTooLarge);
        }
        let mut put = self.client.put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(ByteStream::from(bytes.to_vec()));
        put = match condition {
            S3PutCondition::IfAbsent => put.if_none_match("*"),
            S3PutCondition::IfMatch(etag) => put.if_match(etag),
        };
        let response = put.send().await
            .map_err(|error| classify(error.as_service_error().and_then(|e| e.code())))?;
        // Receipt alone is not a durable baseline. Callers MUST read back,
        // verify BLAKE3 and journal prior to acknowledging publication.
        let etag = response.e_tag()
            .filter(|value| !value.is_empty())
            .ok_or(S3TransportError::CommitUncertain)?;
        Ok(S3ObservedObject { etag: etag.to_owned(), byte_length: bytes.len() as u64 })
    }

    /// Diagnostic listing only! These raw pages do not comprise a stable S3
    /// snapshot; this method must never implement collect_verified_listing.
    pub async fn diagnostic_list_page(
        &self,
        cursor: Option<&str>,
        page_limit: i32,
    ) -> Result<S3DiagnosticPage, S3TransportError> {
        if !(1..=MAX_S3_PAGE_SIZE).contains(&page_limit)
            || cursor.is_some_and(|value| value.is_empty() || value.len() > 8192)
        {
            return Err(S3TransportError::MalformedResponse);
        }
        let prefix = self.keyspace.vault_prefix();
        let result = self.client.list_objects_v2()
            .bucket(&self.bucket)
            .prefix(&prefix)
            .max_keys(page_limit)
            .set_continuation_token(cursor.map(str::to_owned))
            .send().await
            .map_err(|error| classify(error.as_service_error().and_then(|e| e.code())))?;
        let truncated = result.is_truncated()
            .ok_or(S3TransportError::IncompleteListing)?;
        let keys: Vec<String> = result.contents().iter().map(|object| {
            object.key().ok_or(S3TransportError::MalformedResponse)
                .and_then(|key| {
                    self.checked_key(key)?;
                    Ok(key.to_owned())
                })
        }).collect::<Result<_,_>>()?;
        if keys.len() > page_limit as usize {
            return Err(S3TransportError::MalformedResponse);
        }
        let next = result.next_continuation_token().map(str::to_owned);
        if (truncated && next.as_deref().is_none_or(str::is_empty))
            || (!truncated && next.is_some())
        {
            return Err(S3TransportError::IncompleteListing);
        }
        Ok(S3DiagnosticPage { keys, next_cursor: next, complete: !truncated })
    }
}

fn classify(code: Option<&str>) -> S3TransportError {
    match code {
        Some("NoSuchKey" | "NotFound" | "NoSuchBucket") => S3TransportError::NotFound,
        Some("AccessDenied" | "InvalidAccessKeyId" | "SignatureDoesNotMatch") =>
            S3TransportError::PermissionDenied,
        Some("PreconditionFailed") => S3TransportError::PreconditionFailed,
        Some("ConditionalRequestConflict" | "OperationAborted") =>
            S3TransportError::CommitUncertain,
        // For transport drops/timeouts, a mutation can have succeeded: the
        // caller must use head+readback and never blindly retry a PUT.
        _ => S3TransportError::CommitUncertain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::VaultId;

    fn profile(endpoint: Option<&str>) -> S3ConnectionProfile {
        S3ConnectionProfile {
            bucket: "akula-test-bucket".into(),
            region: "us-east-1".into(),
            endpoint: endpoint.map(str::to_owned),
            force_path_style: true,
        }
    }

    #[test]
    fn endpoint_guard_blocks_credential_injection_and_external_plaintext() {
        for endpoint in [
            "http://example.com:9000", "http://127.0.0.1.evil.org:9000",
            "http://10.0.0.1:9000", "https://me:password@example.com",
            "https://example.com/evil", "https://example.com?token=secret",
            "https://example.com/#frag", "file:///tmp/minio",
        ] {
            assert_eq!(profile(Some(endpoint)).validate(), Err(S3TransportError::UnsafeEndpoint), "{endpoint}");
        }
        for endpoint in [
            "https://s3.us-east-1.amazonaws.com",
            "http://127.0.0.1:9000",
            "http://localhost:9000",
        ] {
            assert_eq!(profile(Some(endpoint)).validate(), Ok(()), "{endpoint}");
        }
        let mut unsafe_profile = profile(Some("http://localhost:9000"));
        unsafe_profile.force_path_style = false;
        assert_eq!(unsafe_profile.validate(), Err(S3TransportError::UnsafeEndpoint));
    }

    #[test]
    fn rejects_invalid_buckets_regions_and_guards() {
        for bucket in ["AA", "CAPITAL", "a", "a..b", "bucket/name", "a_b"] {
            let mut invalid = profile(None);
            invalid.bucket = bucket.into();
            assert_eq!(invalid.validate(), Err(S3TransportError::InvalidProfile));
        }
        for region in ["", "../west", "WEST", "r/3", "a b"] {
            let mut invalid = profile(None);
            invalid.region = region.into();
            assert_eq!(invalid.validate(), Err(S3TransportError::InvalidProfile));
        }
        assert_eq!(S3PutCondition::IfMatch(String::new()).validate(), Err(S3TransportError::InvalidProfile));
        assert_eq!(S3PutCondition::IfMatch("\r\n".into()).validate(), Err(S3TransportError::InvalidProfile));
        assert!(S3PutCondition::IfAbsent.validate().is_ok());
    }

    /// Requires an isolated ephemeral MinIO bucket. Never runs on main CI
    /// without explicit opt-in and an externally supplied loopback endpoint.
    #[tokio::test]
    #[ignore = "requires a local, disposable MinIO Docker server"]
    async fn live_minio_conditional_write_and_bounded_read() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").expect("isolated MinIO endpoint required");
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"));
        let ns = S3Keyspace::new("akula-sync/ci", VaultId::new()).unwrap();
        let transport = AwsS3ProbeClient::connect(profile(Some(&endpoint)), ns.clone())
            .await.unwrap();

        // This test bucket is ephemeral and disposable; no production S3
        // credentials are ever used in PR CI.
        transport.client.create_bucket().bucket("akula-test-bucket").send().await.unwrap();
        let a = ns.immutable_manifest_key(super::super::OperationId::new());
        let first = transport.put_conditionally(&a, b"first", S3PutCondition::IfAbsent).await.unwrap();
        let checked = transport.get_bounded(&a, 128).await.unwrap();
        assert_eq!(checked.bytes, b"first");
        assert_eq!(checked.object, first);
        assert_eq!(checked.blake3, blake3::hash(b"first").to_hex().to_string());
        assert_eq!(
            transport.put_conditionally(&a, b"collision", S3PutCondition::IfAbsent).await,
            Err(S3TransportError::PreconditionFailed)
        );
        assert_eq!(
            transport.put_conditionally(&a, b"stale", S3PutCondition::IfMatch("\"wrong\"".into())).await,
            Err(S3TransportError::PreconditionFailed)
        );
        let second = transport.put_conditionally(&a, b"second", S3PutCondition::IfMatch(first.etag.clone())).await.unwrap();
        assert_ne!(second.etag, first.etag);
        assert_eq!(transport.get_bounded(&a, 10).await.unwrap().bytes, b"second");
        assert_eq!(transport.get_bounded(&a, 5).await, Err(S3TransportError::ObjectTooLarge));
        assert_eq!(transport.head(&ns.immutable_manifest_key(super::super::OperationId::new())).await,
            Err(S3TransportError::NotFound));
    }

    #[tokio::test]
    #[ignore = "requires local MinIO Docker and disposable credentials"]
    async fn live_minio_diagnostic_pagination_is_not_a_remote_snapshot() {
        let endpoint = std::env::var("AQ_S3_TEST_ENDPOINT").unwrap();
        assert!(endpoint.starts_with("http://127.0.0.1:")
            || endpoint.starts_with("http://localhost:"));
        let ns = S3Keyspace::new("akula-sync/paging", VaultId::new()).unwrap();
        let transport = AwsS3ProbeClient::connect(profile(Some(&endpoint)), ns.clone())
            .await.unwrap();
        // Bucket created by the first test, when this case is run on the
        // same runner with --test-threads=1.
        for _ in 0..13 {
            let key = ns.immutable_manifest_key(super::super::OperationId::new());
            transport.put_conditionally(&key, b"data", S3PutCondition::IfAbsent).await.unwrap();
        }
        let mut cursor = None;
        let mut seen = Vec::new();
        loop {
            let page = transport.diagnostic_list_page(cursor.as_deref(), 5).await.unwrap();
            seen.extend(page.keys);
            if page.complete { break; }
            cursor = page.next_cursor;
        }
        assert_eq!(seen.len(), 13);
        assert_eq!(seen.iter().collect::<std::collections::BTreeSet<_>>().len(), 13);
    }
}
