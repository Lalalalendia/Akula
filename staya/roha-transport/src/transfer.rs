//! Bounded, integrity-checked object chunks. Verification is in RAM, not a
//! durable receipt or permission to mutate files via Akula's File Gate.

pub const MAX_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_OBJECT_BYTES: usize = 16 * 1024 * 1024;
pub const FRAME_HEADER_BYTES: usize = 92;
const MAGIC: &[u8; 8] = b"AKSTAY01";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransferError {
    ObjectTooLarge,
    ChunkTooLarge,
    EmptyChunk,
    InvalidRange,
    Truncated,
    InvalidMarker,
    FrameLength,
    ManifestMismatch,
    OutOfOrder,
    IntegrityMismatch,
    AlreadyComplete,
}

impl std::fmt::Display for TransferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for TransferError {}

/// Expected file metadata must originate from an authorized Akula catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferManifest {
    length: u64,
    digest: [u8; 32],
}

impl TransferManifest {
    pub fn for_bytes(bytes: &[u8]) -> Result<Self, TransferError> {
        if bytes.len() > MAX_OBJECT_BYTES {
            return Err(TransferError::ObjectTooLarge);
        }
        Ok(Self {
            length: bytes.len() as u64,
            digest: *blake3::hash(bytes).as_bytes(),
        })
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    /// Encode marker, total size, offset, chunk length, file digest, chunk
    /// digest, and bytes. Frames cannot be used to request filesystem writes.
    pub fn encode_chunk(&self, offset: u64, data: &[u8]) -> Result<Vec<u8>, TransferError> {
        if self.length > MAX_OBJECT_BYTES as u64 {
            return Err(TransferError::ObjectTooLarge);
        }
        if data.len() > MAX_CHUNK_BYTES {
            return Err(TransferError::ChunkTooLarge);
        }
        if data.is_empty() && self.length != 0 {
            return Err(TransferError::EmptyChunk);
        }
        let end = offset
            .checked_add(data.len() as u64)
            .ok_or(TransferError::InvalidRange)?;
        if end > self.length || (data.is_empty() && offset != 0) {
            return Err(TransferError::InvalidRange);
        }

        let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + data.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&self.length.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.digest);
        out.extend_from_slice(blake3::hash(data).as_bytes());
        out.extend_from_slice(data);
        Ok(out)
    }
}

/// Staged, hash-checked RAM data; not persistent across an application restart.
#[derive(Debug)]
pub struct VerifiedAssembly {
    manifest: TransferManifest,
    bytes: Vec<u8>,
    complete: bool,
}

impl VerifiedAssembly {
    pub fn new(manifest: TransferManifest) -> Result<Self, TransferError> {
        if manifest.length > MAX_OBJECT_BYTES as u64 {
            return Err(TransferError::ObjectTooLarge);
        }
        Ok(Self {
            manifest,
            bytes: Vec::new(),
            complete: false,
        })
    }

    pub fn verified_offset(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// A complete, verified buffer; a File Gate receipt is still required.
    pub fn complete_bytes(&self) -> Option<&[u8]> {
        self.complete.then_some(self.bytes.as_slice())
    }

    pub fn accept_frame(&mut self, frame: &[u8]) -> Result<u64, TransferError> {
        if self.complete {
            return Err(TransferError::AlreadyComplete);
        }
        if frame.len() < FRAME_HEADER_BYTES {
            return Err(TransferError::Truncated);
        }
        if &frame[..8] != MAGIC {
            return Err(TransferError::InvalidMarker);
        }
        let total = u64::from_le_bytes(frame[8..16].try_into().unwrap());
        let offset = u64::from_le_bytes(frame[16..24].try_into().unwrap());
        let length = u32::from_le_bytes(frame[24..28].try_into().unwrap()) as usize;
        let bytes = &frame[FRAME_HEADER_BYTES..];

        if total > MAX_OBJECT_BYTES as u64 {
            return Err(TransferError::ObjectTooLarge);
        }
        if length > MAX_CHUNK_BYTES {
            return Err(TransferError::ChunkTooLarge);
        }
        if length != bytes.len() {
            return Err(TransferError::FrameLength);
        }
        if total != self.manifest.length || frame[28..60] != self.manifest.digest {
            return Err(TransferError::ManifestMismatch);
        }
        if offset != self.verified_offset() {
            return Err(TransferError::OutOfOrder);
        }
        if bytes.is_empty() && total != 0 {
            return Err(TransferError::EmptyChunk);
        }
        let end = offset
            .checked_add(length as u64)
            .ok_or(TransferError::InvalidRange)?;
        if end > total {
            return Err(TransferError::InvalidRange);
        }
        if frame[60..92] != *blake3::hash(bytes).as_bytes() {
            return Err(TransferError::IntegrityMismatch);
        }

        if end == total {
            let mut full = blake3::Hasher::new();
            full.update(&self.bytes);
            full.update(bytes);
            if full.finalize().as_bytes() != &self.manifest.digest {
                return Err(TransferError::IntegrityMismatch);
            }
        }
        self.bytes.extend_from_slice(bytes);
        self.complete = end == total;
        Ok(self.verified_offset())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_empty_and_chunked_objects() {
        for data in [vec![], vec![17], vec![99; MAX_CHUNK_BYTES + 17]] {
            let m = TransferManifest::for_bytes(&data).unwrap();
            let mut stage = VerifiedAssembly::new(m).unwrap();
            if data.is_empty() {
                assert_eq!(stage.accept_frame(&m.encode_chunk(0, &[]).unwrap()), Ok(0));
            } else {
                for (i, bytes) in data.chunks(MAX_CHUNK_BYTES).enumerate() {
                    let offset = (i * MAX_CHUNK_BYTES) as u64;
                    assert_eq!(
                        stage.accept_frame(&m.encode_chunk(offset, bytes).unwrap()),
                        Ok(offset + bytes.len() as u64)
                    );
                }
            }
            assert_eq!(stage.complete_bytes(), Some(data.as_slice()));
            assert_eq!(stage.accept_frame(&[]), Err(TransferError::AlreadyComplete));
        }
    }

    #[test]
    fn refuses_corrupted_replayed_or_out_of_order_chunks_without_data_loss() {
        let data = vec![41; MAX_CHUNK_BYTES + 3];
        let m = TransferManifest::for_bytes(&data).unwrap();
        let mut stage = VerifiedAssembly::new(m).unwrap();
        let a = m.encode_chunk(0, &data[..MAX_CHUNK_BYTES]).unwrap();
        let b = m
            .encode_chunk(MAX_CHUNK_BYTES as u64, &data[MAX_CHUNK_BYTES..])
            .unwrap();
        assert_eq!(stage.accept_frame(&b), Err(TransferError::OutOfOrder));
        let mut corrupt = a.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert_eq!(
            stage.accept_frame(&corrupt),
            Err(TransferError::IntegrityMismatch)
        );
        assert_eq!(stage.verified_offset(), 0);
        let foreign = TransferManifest::for_bytes(&vec![42; data.len()]).unwrap();
        assert_eq!(
            stage.accept_frame(&foreign.encode_chunk(0, &data[..MAX_CHUNK_BYTES]).unwrap()),
            Err(TransferError::ManifestMismatch)
        );
        assert_eq!(stage.accept_frame(&a), Ok(MAX_CHUNK_BYTES as u64));
        assert_eq!(stage.accept_frame(&a), Err(TransferError::OutOfOrder));
        assert_eq!(stage.accept_frame(&b), Ok(data.len() as u64));
        assert_eq!(stage.complete_bytes(), Some(data.as_slice()));
    }

    #[test]
    fn rejects_oversized_and_malformed_frames() {
        assert_eq!(
            TransferManifest::for_bytes(&vec![0; MAX_OBJECT_BYTES + 1]),
            Err(TransferError::ObjectTooLarge)
        );
        let m = TransferManifest::for_bytes(&vec![1; MAX_CHUNK_BYTES + 1]).unwrap();
        assert_eq!(
            m.encode_chunk(0, &vec![1; MAX_CHUNK_BYTES + 1]),
            Err(TransferError::ChunkTooLarge)
        );
        let mut stage = VerifiedAssembly::new(m).unwrap();
        assert_eq!(stage.accept_frame(&[0; 10]), Err(TransferError::Truncated));
        let mut bad = m.encode_chunk(0, &[1; 10]).unwrap();
        bad[0] ^= 1;
        assert_eq!(stage.accept_frame(&bad), Err(TransferError::InvalidMarker));
        assert_eq!(stage.verified_offset(), 0);
    }
}
