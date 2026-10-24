//! PUBLIC-S3-CDN-NATIVE-01: non-shipping, read-only native source-selection seam.
//!
//! This deliberately lives in the EXISTING Akula/Staya interop crate. It does
//! not add a QUIC listener, ALPN, device identity, S3 account or a second Sync
//! Core. Public content is not a Vault file and creates no File Gate authority.
//!
//! Web reader v1: SHA-256 of complete object (ContentID) and each chunk.
//! Staya's ContentId: BLAKE3 of complete object, NEVER the same identifier.
//! Both are independently checked in the bounded streaming receipt.
//!
//! Manifest trust MUST come from an independently pinned signed catalog. A
//! peer-supplied hash, an S3 ETag or an unsigned manifest is NOT a trust root.

use iroh::endpoint::Connection;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::Write;

const MAX_MANIFEST_BYTES: usize = 8 * 1024 * 1024;
const MAX_OBJECT_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_PARTS: usize = 65_536;
const MAX_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
const MIN_CHUNK_BYTES: u64 = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicReadError {
    UntrustedManifest,
    InvalidManifest,
    MissingOrigin,
    BadOriginResponse,
    CorruptOriginChunk,
    WrongFullDigest,
    OutputWriteFailed,
    InvalidChunkIndex,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestV1 {
    version: u32,
    content_id: String,
    name: String,
    size: u64,
    chunk_size: u64,
    origin_url: String,
    chunks: Vec<ChunkV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkV1 {
    offset: u64,
    size: u64,
    sha256: String,
}

/// A parsed public manifest whose exact JSON bytes were validated against a
/// caller-supplied, independent digest from an authenticated publisher index.
/// This verifies a digest pin, NOT an Ed25519 signature by itself.
pub struct PublisherPinnedManifest {
    data: ManifestV1,
    content_id_bytes: [u8; 32],
}

fn hex_digest(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut result = [0_u8; 32];
    for (i, slot) in result.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(result)
}

fn valid_origin(url: &str) -> bool {
    if url.len() > 4096 || url.chars().any(char::is_control) {
        return false;
    }
    if let Some(authority) = url.strip_prefix("https://") {
        let host = authority.split(['/', '?', '#']).next().unwrap_or("");
        return !host.is_empty() && !host.contains('@') && !host.contains('\\')
            && !host.chars().any(char::is_whitespace);
    }
    // Only the test harness may use this path. A production S3 adapter MUST
    // independently allowlist its HTTPS origin; it never follows arbitrary URLs.
    url.starts_with("/origin/") && !url.contains("..") && !url.contains('?')
}

impl PublisherPinnedManifest {
    pub fn from_pinned_json(raw: &[u8], independent_sha256: &str) -> Result<Self, PublicReadError> {
        if raw.is_empty() || raw.len() > MAX_MANIFEST_BYTES {
            return Err(PublicReadError::InvalidManifest);
        }
        let pin = hex_digest(independent_sha256).ok_or(PublicReadError::UntrustedManifest)?;
        let actual = Sha256::digest(raw);
        if actual[..] != pin[..] {
            return Err(PublicReadError::UntrustedManifest);
        }
        let data: ManifestV1 =
            serde_json::from_slice(raw).map_err(|_| PublicReadError::InvalidManifest)?;
        let content_id_bytes =
            hex_digest(&data.content_id).ok_or(PublicReadError::InvalidManifest)?;
        if data.version != 1
            || data.size == 0
            || data.size > MAX_OBJECT_BYTES
            || !(MIN_CHUNK_BYTES..=MAX_CHUNK_BYTES).contains(&data.chunk_size)
            || data.chunks.is_empty()
            || data.chunks.len() > MAX_PARTS
            || data.name.is_empty()
            || data.name.len() > 255
            || data.name.chars().any(char::is_control)
            || data.name.contains(['/', '\\'])
            || data.name == "."
            || data.name == ".."
            || !valid_origin(&data.origin_url)
        {
            return Err(PublicReadError::InvalidManifest);
        }
        let mut next = 0_u64;
        for chunk in &data.chunks {
            if chunk.offset != next
                || chunk.size == 0
                || chunk.size > data.chunk_size
                || hex_digest(&chunk.sha256).is_none()
            {
                return Err(PublicReadError::InvalidManifest);
            }
            let Some(end) = next.checked_add(chunk.size) else {
                return Err(PublicReadError::InvalidManifest);
            };
            if end > data.size || chunk.size != (data.size - next).min(data.chunk_size) {
                return Err(PublicReadError::InvalidManifest);
            }
            next = end;
        }
        if next != data.size {
            return Err(PublicReadError::InvalidManifest);
        }
        Ok(Self { data, content_id_bytes })
    }

    pub fn content_id_sha256(&self) -> &str {
        &self.data.content_id
    }

    pub fn object_size(&self) -> u64 {
        self.data.size
    }

    pub fn chunk_count(&self) -> usize {
        self.data.chunks.len()
    }

    pub fn origin_url_for_allowlisted_adapter(&self) -> &str {
        &self.data.origin_url
    }

    pub fn request(&self, index: usize) -> Result<ChunkRequest<'_>, PublicReadError> {
        let chunk = self.data.chunks.get(index).ok_or(PublicReadError::InvalidChunkIndex)?;
        Ok(ChunkRequest {
            content_id_sha256: &self.data.content_id,
            index,
            offset: chunk.offset,
            length: usize::try_from(chunk.size).map_err(|_| PublicReadError::InvalidManifest)?,
            object_size: self.data.size,
            sha256: &chunk.sha256,
        })
    }
}

/// A single independently verified range. This is an identifier for a
/// PUBLIC object, not a private Vault grant or a Staya Blob/ContentId.
#[derive(Clone, Copy, Debug)]
pub struct ChunkRequest<'a> {
    pub content_id_sha256: &'a str,
    pub index: usize,
    pub offset: u64,
    pub length: usize,
    pub object_size: u64,
    pub sha256: &'a str,
}

/// Native local cache / Staya peer API. Implementations MUST bound transport
/// responses to request.length BEFORE allocating. A failed/absent candidate
/// is advisory; the caller will try the next source. No File Gate writes.
pub trait OptionalChunkSource {
    fn try_chunk(&mut self, request: &ChunkRequest<'_>) -> Result<Option<Vec<u8>>, ()>;
}

/// Backend supplied by the ALREADY existing Staya/Iroh session. No wire format,
/// protocol listener or alternate QUIC transport is defined in this module.
/// A concrete Bao/Staya public-read backend still needs an independent E2E gate.
pub trait ExistingStayaIrohPublicReader {
    fn try_staya_public_chunk(
        &mut self,
        authenticated_connection: &Connection,
        request: &ChunkRequest<'_>,
    ) -> Result<Option<Vec<u8>>, ()>;
}

/// Type-level bridge to the existing Iroh connection. The remote peer identity
/// comes from the TLS/QUIC connection, never from an untrusted announcement.
/// Merely having a live connection does NOT grant private File Gate access.
pub struct ExistingIrohPeerSource<'a, T: ExistingStayaIrohPublicReader> {
    pub connection: &'a Connection,
    pub staya_reader: &'a mut T,
}

impl<T: ExistingStayaIrohPublicReader> OptionalChunkSource for ExistingIrohPeerSource<'_, T> {
    fn try_chunk(&mut self, request: &ChunkRequest<'_>) -> Result<Option<Vec<u8>>, ()> {
        self.staya_reader.try_staya_public_chunk(self.connection, request)
    }
}

impl<T: ExistingStayaIrohPublicReader> ExistingIrohPeerSource<'_, T> {
    pub fn authenticated_remote_id(&self) -> [u8; 32] {
        *self.connection.remote_id().as_bytes()
    }
}

/// The S3 HTTPS adapter must enforce status, Content-Range and size *while
/// streaming*, and cap accumulated bytes to request.length. This contract
/// rechecks metadata and SHA-256 before returning any bytes to the caller.
pub struct OriginReply {
    pub status: u16,
    pub content_range: Option<String>,
    pub content_length: Option<u64>,
    pub content_encoding: Option<String>,
    pub bytes: Vec<u8>,
}

pub trait ImmutableRangeOrigin {
    fn get_range(&mut self, request: &ChunkRequest<'_>) -> Result<OriginReply, ()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceKind {
    Cache,
    StayaPeer,
    S3Origin,
}

/// Cannot be manufactured from unchecked bytes outside this module.
pub struct VerifiedPublicPart {
    request_index: usize,
    source: SourceKind,
    bytes: Vec<u8>,
}

impl VerifiedPublicPart {
    pub fn index(&self) -> usize {
        self.request_index
    }

    pub fn source(&self) -> SourceKind {
        self.source
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceMetrics {
    pub verified_cache_bytes: u64,
    pub verified_peer_bytes: u64,
    pub verified_origin_bytes: u64,
    pub rejected_cache_chunks: u64,
    pub rejected_peer_chunks: u64,
    pub origin_requests: u64,
    pub origin_received_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedStreamReceipt {
    /// This is the web manifest SHA-256 identity.
    pub object_sha256: [u8; 32],
    /// This is the *separately calculated* Staya BLAKE3/Bao identity.
    pub staya_content_id: staya_transfer::ContentId,
    pub bytes: u64,
    pub metrics: SourceMetrics,
}

/// A read-only coordinator. Local cache and P2P data remain untrusted until
/// verified. Fallback is per chunk, never an implicit migration into Vault.
pub struct PublicObjectReader<'a> {
    pub manifest: &'a PublisherPinnedManifest,
    pub cache: &'a mut dyn OptionalChunkSource,
    pub peer: &'a mut dyn OptionalChunkSource,
    pub origin: &'a mut dyn ImmutableRangeOrigin,
    metrics: SourceMetrics,
}

fn matches_chunk(request: &ChunkRequest<'_>, bytes: &[u8]) -> bool {
    if bytes.len() != request.length {
        return false;
    }
    let actual = Sha256::digest(bytes);
    match hex_digest(request.sha256) {
        Some(expected) => actual[..] == expected[..],
        None => false,
    }
}

impl PublicObjectReader<'_> {
    pub fn new<'a>(
        manifest: &'a PublisherPinnedManifest,
        cache: &'a mut dyn OptionalChunkSource,
        peer: &'a mut dyn OptionalChunkSource,
        origin: &'a mut dyn ImmutableRangeOrigin,
    ) -> PublicObjectReader<'a> {
        PublicObjectReader { manifest, cache, peer, origin, metrics: SourceMetrics::default() }
    }

    pub fn metrics(&self) -> SourceMetrics {
        self.metrics
    }

    pub fn read_part(&mut self, index: usize) -> Result<VerifiedPublicPart, PublicReadError> {
        let request = self.manifest.request(index)?;
        match self.cache.try_chunk(&request) {
            Ok(Some(bytes)) if matches_chunk(&request, &bytes) => {
                self.metrics.verified_cache_bytes += bytes.len() as u64;
                return Ok(VerifiedPublicPart { request_index: index, source: SourceKind::Cache, bytes });
            }
            Ok(Some(_)) => self.metrics.rejected_cache_chunks += 1,
            _ => {}
        }
        match self.peer.try_chunk(&request) {
            Ok(Some(bytes)) if matches_chunk(&request, &bytes) => {
                self.metrics.verified_peer_bytes += bytes.len() as u64;
                return Ok(VerifiedPublicPart { request_index: index, source: SourceKind::StayaPeer, bytes });
            }
            Ok(Some(_)) => self.metrics.rejected_peer_chunks += 1,
            _ => {}
        }
        self.metrics.origin_requests += 1;
        let reply = self.origin.get_range(&request).map_err(|_| PublicReadError::MissingOrigin)?;
        self.metrics.origin_received_bytes += reply.bytes.len() as u64;
        let end = request.offset + (request.length as u64) - 1;
        let expected_range = format!("bytes {}-{}/{}", request.offset, end, request.object_size);
        if reply.status != 206
            || reply.content_range.as_deref() != Some(expected_range.as_str())
            || reply.content_length.is_some_and(|n| n != request.length as u64)
            || reply.content_encoding.as_deref().is_some_and(|enc| enc != "identity")
        {
            return Err(PublicReadError::BadOriginResponse);
        }
        if !matches_chunk(&request, &reply.bytes) {
            return Err(PublicReadError::CorruptOriginChunk);
        }
        self.metrics.verified_origin_bytes += reply.bytes.len() as u64;
        Ok(VerifiedPublicPart { request_index: index, source: SourceKind::S3Origin, bytes: reply.bytes })
    }

    /// Writes at most ONE verified <=4 MiB part at once. The caller MUST
    /// supply a private temporary staging file and publish only on Ok(receipt).
    /// No private Vault, existing path, or user-visible final file is opened.
    pub fn stream_into(&mut self, output: &mut impl Write) -> Result<VerifiedStreamReceipt, PublicReadError> {
        let mut sha = Sha256::new();
        let mut blake = blake3::Hasher::new();
        let mut total = 0_u64;
        for index in 0..self.manifest.chunk_count() {
            let part = self.read_part(index)?;
            sha.update(part.bytes());
            blake.update(part.bytes());
            output.write_all(part.bytes()).map_err(|_| PublicReadError::OutputWriteFailed)?;
            total += part.bytes().len() as u64;
        }
        let full = sha.finalize();
        if full[..] != self.manifest.content_id_bytes[..] || total != self.manifest.object_size() {
            return Err(PublicReadError::WrongFullDigest);
        }
        Ok(VerifiedStreamReceipt {
            object_sha256: self.manifest.content_id_bytes,
            staya_content_id: staya_transfer::ContentId(*blake.finalize().as_bytes()),
            bytes: total,
            metrics: self.metrics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn hx(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn contents() -> Vec<u8> {
        (0..(3 * 1024 + 19)).map(|n| (n % 251) as u8).collect()
    }

    fn sample_manifest(bytes: &[u8]) -> (Vec<u8>, String) {
        let chunks = bytes.chunks(1024).enumerate().map(|(index, chunk)| {
            serde_json::json!({
                "offset": index * 1024, "size": chunk.len(), "sha256": hx(chunk)
            })
        }).collect::<Vec<_>>();
        let json = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "content_id": hx(bytes),
            "name": "public-example.bin",
            "size": bytes.len(),
            "chunk_size": 1024,
            "origin_url": "https://public.example.test/v1/immutable.bin",
            "chunks": chunks
        })).unwrap();
        let pin = hx(&json);
        (json, pin)
    }

    fn sample(bytes: &[u8]) -> PublisherPinnedManifest {
        let (json, pin) = sample_manifest(bytes);
        PublisherPinnedManifest::from_pinned_json(&json, &pin).unwrap()
    }

    struct MemorySource {
        data: Vec<u8>,
        available: BTreeSet<usize>,
        corrupt: BTreeSet<usize>,
        calls: usize,
    }

    impl MemorySource {
        fn new(bytes: &[u8], indexes: &[usize], corrupt: &[usize]) -> Self {
            Self { data: bytes.to_vec(), available: indexes.iter().copied().collect(),
                corrupt: corrupt.iter().copied().collect(), calls: 0 }
        }
    }

    impl OptionalChunkSource for MemorySource {
        fn try_chunk(&mut self, request: &ChunkRequest<'_>) -> Result<Option<Vec<u8>>, ()> {
            self.calls += 1;
            if !self.available.contains(&request.index) {
                return Ok(None);
            }
            let a = request.offset as usize;
            let mut buf = self.data[a..a + request.length].to_vec();
            if self.corrupt.contains(&request.index) {
                buf[0] ^= 0xff;
            }
            Ok(Some(buf))
        }
    }

    struct MemoryOrigin {
        data: Vec<u8>,
        status: u16,
        bad_range: bool,
        corrupt: bool,
        calls: usize,
    }

    impl MemoryOrigin {
        fn new(bytes: &[u8]) -> Self {
            Self { data: bytes.to_vec(), status: 206, bad_range: false, corrupt: false, calls: 0 }
        }
    }

    impl ImmutableRangeOrigin for MemoryOrigin {
        fn get_range(&mut self, request: &ChunkRequest<'_>) -> Result<OriginReply, ()> {
            self.calls += 1;
            let a = request.offset as usize;
            let mut body = self.data[a..a + request.length].to_vec();
            if self.corrupt {
                body[0] ^= 0xff;
            }
            let end = request.offset + request.length as u64 - 1;
            Ok(OriginReply {
                status: self.status,
                content_range: Some(if self.bad_range {
                    "bytes 0-0/1000".into()
                } else {
                    format!("bytes {}-{end}/{}", request.offset, request.object_size)
                }),
                content_length: Some(body.len() as u64),
                content_encoding: None,
                bytes: body,
            })
        }
    }

    #[test]
    fn web_sha256_and_staya_blake3_are_separately_verified() {
        let data = contents();
        let manifest = sample(&data);
        let mut cache = MemorySource::new(&data, &[0], &[]);
        let mut peer = MemorySource::new(&data, &[1, 2], &[]);
        let mut origin = MemoryOrigin::new(&data);
        let mut reader = PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin);
        let mut staging = Vec::new();
        let receipt = reader.stream_into(&mut staging).unwrap();
        assert_eq!(staging, data);
        assert_eq!(receipt.bytes, data.len() as u64);
        assert_eq!(receipt.object_sha256.to_vec(), Sha256::digest(&data).to_vec());
        assert_eq!(receipt.staya_content_id.0, *blake3::hash(&data).as_bytes());
        assert_eq!(receipt.metrics.verified_cache_bytes, 1024);
        assert_eq!(receipt.metrics.verified_peer_bytes, 2048);
        assert_eq!(receipt.metrics.verified_origin_bytes, 19);
        assert_eq!(receipt.metrics.origin_requests, 1);
    }

    #[test]
    fn malformed_or_peer_rewritten_publisher_manifest_fails_closed() {
        let data = contents();
        let (json, pin) = sample_manifest(&data);
        assert!(matches!(
            PublisherPinnedManifest::from_pinned_json(&json, &hx(b"peer-supplied-value")),
            Err(PublicReadError::UntrustedManifest)
        ));
        let mut wrong: serde_json::Value = serde_json::from_slice(&json).unwrap();
        wrong["content_id"] = serde_json::json!(hx(b"untrusted replacement"));
        let bytes = serde_json::to_vec(&wrong).unwrap();
        assert!(matches!(
            PublisherPinnedManifest::from_pinned_json(&bytes, &pin),
            Err(PublicReadError::UntrustedManifest)
        ));
        let mut extra: serde_json::Value = serde_json::from_slice(&json).unwrap();
        extra["grant"] = serde_json::json!("vault-write");
        let bytes = serde_json::to_vec(&extra).unwrap();
        assert!(matches!(
            PublisherPinnedManifest::from_pinned_json(&bytes, &hx(&bytes)),
            Err(PublicReadError::InvalidManifest)
        ));
    }

    #[test]
    fn rejects_gap_wrong_part_size_foreign_url_and_dangerous_file_name() {
        let data = contents();
        let (json, _) = sample_manifest(&data);
        for (field, value) in [
            ("chunks", serde_json::json!([{"offset": 1, "size": data.len(), "sha256": hx(&data)}])),
            ("origin_url", serde_json::json!("file:///etc/passwd")),
            ("name", serde_json::json!("../test.bin")),
            ("chunk_size", serde_json::json!(0)),
            ("version", serde_json::json!(2)),
        ] {
            let mut altered: serde_json::Value = serde_json::from_slice(&json).unwrap();
            altered[field] = value;
            let encoded = serde_json::to_vec(&altered).unwrap();
            assert!(matches!(
                PublisherPinnedManifest::from_pinned_json(&encoded, &hx(&encoded)),
                Err(PublicReadError::InvalidManifest)
            ), "field: {field}");
        }
    }

    #[test]
    fn bad_cache_and_hostile_peer_are_replaced_from_origin() {
        let data = contents();
        let manifest = sample(&data);
        let mut cache = MemorySource::new(&data, &[0], &[0]);
        let mut peer = MemorySource::new(&data, &[0], &[0]);
        let mut origin = MemoryOrigin::new(&data);
        let mut reader = PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin);
        let first = reader.read_part(0).unwrap();
        assert_eq!(first.source(), SourceKind::S3Origin);
        assert_eq!(first.bytes(), &data[..1024]);
        assert_eq!(reader.metrics().rejected_cache_chunks, 1);
        assert_eq!(reader.metrics().rejected_peer_chunks, 1);
        assert_eq!(reader.metrics().verified_origin_bytes, 1024);
    }

    #[test]
    fn origin_http_200_and_forged_content_range_are_not_accepted() {
        let data = contents();
        let manifest = sample(&data);
        let mut cache = MemorySource::new(&data, &[], &[]);
        let mut peer = MemorySource::new(&data, &[], &[]);
        let mut origin = MemoryOrigin::new(&data);
        origin.status = 200;
        assert!(matches!(
            PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin).read_part(0),
            Err(PublicReadError::BadOriginResponse)
        ));
        origin.status = 206;
        origin.bad_range = true;
        assert!(matches!(
            PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin).read_part(0),
            Err(PublicReadError::BadOriginResponse)
        ));
    }

    #[test]
    fn corrupted_origin_and_out_of_range_are_denied() {
        let data = contents();
        let manifest = sample(&data);
        let mut cache = MemorySource::new(&data, &[], &[]);
        let mut peer = MemorySource::new(&data, &[], &[]);
        let mut origin = MemoryOrigin::new(&data);
        origin.corrupt = true;
        assert!(matches!(
            PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin).read_part(0),
            Err(PublicReadError::CorruptOriginChunk)
        ));
        assert!(matches!(
            PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin).read_part(99999),
            Err(PublicReadError::InvalidChunkIndex)
        ));
    }

    #[test]
    fn wrong_full_digest_never_returns_verified_receipt() {
        let data = contents();
        let (json, _) = sample_manifest(&data);
        let mut altered: serde_json::Value = serde_json::from_slice(&json).unwrap();
        altered["content_id"] = serde_json::json!(hx(b"incorrect expected full hash"));
        let bytes = serde_json::to_vec(&altered).unwrap();
        let manifest = PublisherPinnedManifest::from_pinned_json(&bytes, &hx(&bytes)).unwrap();
        let mut cache = MemorySource::new(&data, &[], &[]);
        let mut peer = MemorySource::new(&data, &[], &[]);
        let mut origin = MemoryOrigin::new(&data);
        let mut scratch = Vec::new();
        assert!(matches!(
            PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin)
                .stream_into(&mut scratch),
            Err(PublicReadError::WrongFullDigest)
        ));
        // Scratch bytes exist but no receipt: File Gate MUST NOT publish them.
        assert_eq!(scratch, data);
    }

    #[test]
    fn wrong_output_sink_fails_without_verified_receipt() {
        struct FailingSink;
        impl Write for FailingSink {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk is full"))
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let data = contents();
        let manifest = sample(&data);
        let mut cache = MemorySource::new(&data, &[], &[]);
        let mut peer = MemorySource::new(&data, &[], &[]);
        let mut origin = MemoryOrigin::new(&data);
        assert!(matches!(
            PublicObjectReader::new(&manifest, &mut cache, &mut peer, &mut origin)
                .stream_into(&mut FailingSink),
            Err(PublicReadError::OutputWriteFailed)
        ));
    }
}
