//! Real Iroh QUIC / iroh-blobs Bao ranged GET over an EXISTING connection.
//! No new ALPN, listener, identity or private Vault permission is created.
//! The BLAKE3 object root must be pinned independently, never peer-supplied.
//! This is a lab-only async operation, not a synchronous transport shim.
use crate::public_cdn_lab::ChunkRequest;
use iroh::endpoint::Connection;
use iroh_blobs::{
    Hash,
    protocol::{ChunkRanges, ChunkRangesExt, GetRequest, Request},
};
use staya_transfer::{
    ByteRange, ContentId, Manifest, TransferRequest,
    bao::{verify_bao_range, MAX_BAO_PROOF_BYTES},
};
use std::time::Duration;

pub struct PinnedBaoIroh<'a> {
    connection: &'a Connection,
    root_blake3: [u8; 32],
    object_size: u64,
    /// An independently trusted operator/public publisher permission gate.
    /// The provider must still independently enforce its own public-root ACL.
    public_read_enabled: bool,
}

impl<'a> PinnedBaoIroh<'a> {
    pub fn new(
        authenticated_connection: &'a Connection,
        independently_pinned_root: [u8; 32],
        object_size: u64,
        explicit_public_permission: bool,
    ) -> Result<Self, &'static str> {
        if !explicit_public_permission || object_size == 0 {
            return Err("public read is not authorized by trusted local policy");
        }
        Ok(Self {
            connection: authenticated_connection,
            root_blake3: independently_pinned_root,
            object_size,
            public_read_enabled: true,
        })
    }

    pub fn authenticated_peer(&self) -> [u8; 32] {
        *self.connection.remote_id().as_bytes()
    }

    /// Read one authenticated Bao range (max 1 MiB), then also validate the
    /// public SHA-256 chunk identity. It only returns bytes, no file privilege.
    pub async fn read_public_part(
        &mut self,
        request: &ChunkRequest<'_>,
    ) -> Result<Vec<u8>, &'static str> {
        if !self.public_read_enabled
            || request.object_size != self.object_size
            || request.length == 0
            || request.length > 1024 * 1024
        {
            return Err("invalid bounded public read");
        }
        let end = request.offset.checked_add(request.length as u64)
            .ok_or("overflow")?;
        if end > self.object_size {
            return Err("outside pinned object");
        }
        let blob_hash = Hash::from_bytes(self.root_blake3);
        let message = postcard::to_stdvec(&Request::Get(
            GetRequest::blob_ranges(
                blob_hash,
                ChunkRanges::bytes(request.offset..end),
            )
        )).map_err(|_| "cannot serialize Bao request")?;

        let exchange = async {
            let (mut send, mut recv) = self.connection.open_bi().await
                .map_err(|_| "cannot open existing QUIC connection")?;
            send.write_all(&message).await.map_err(|_| "Bao request send failed")?;
            send.finish().map_err(|_| "Bao request finish failed")?;
            recv.read_to_end(MAX_BAO_PROOF_BYTES + 8 + 1).await
                .map_err(|_| "invalid bounded Bao proof response")
        };
        let response = tokio::time::timeout(Duration::from_secs(25), exchange)
            .await.map_err(|_| "Bao GET timed out")??;
        // iroh-blobs prefixes an actual Bao GET response with the
        // 8-byte little-endian FULL object length. It is not Bao proof data.
        if response.len() < 8 || response.len() > MAX_BAO_PROOF_BYTES + 8 {
            return Err("Bao frame over resource budget");
        }
        let wire_size = u64::from_le_bytes(
            response[..8].try_into().map_err(|_| "truncated Bao length prefix")?
        );
        if wire_size != self.object_size {
            return Err("Bao framed object size disagrees with pinned manifest");
        }
        let manifest = Manifest {
            id: ContentId(self.root_blake3),
            size: self.object_size,
        };
        let range = ByteRange::new(request.offset, end, manifest.size)
            .map_err(|_| "invalid Bao range")?;
        let proof_request = TransferRequest::new(manifest, range)
            .map_err(|_| "invalid Bao manifest")?;
        let verified = verify_bao_range(&proof_request, &response[8..])
            .map_err(|_| "Bao cryptographic verification failed")?;
        let bytes = verified.into_bytes();
        // Independently pin browser SHA-256 even for a valid BLAKE3 proof.
        use sha2::{Digest, Sha256};
        let sha = Sha256::digest(&bytes);
        let actual = sha.iter().map(|b| format!("{b:02x}")).collect::<String>();
        if actual != request.sha256 || bytes.len() != request.length {
            return Err("public SHA-256 chunk mismatch");
        }
        Ok(bytes)
    }
}
