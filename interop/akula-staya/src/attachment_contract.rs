//! Inert Akula Core -> real Staya adapter. No source paths, read permissions,
//! network listeners, or Vault writes are granted by this metadata mapping.
use aquilum_core::p2p_attachment_contract::AttachmentNoticeV1;
use staya_transfer::{ByteRange, ContentId, Manifest, TransferRequest, TransferError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachmentBridgeError {
    InvalidRootOrSize,
    InvalidResume,
    Staya(TransferError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckedResume {
    pub offset: u64,
    pub verified_prefix: [u8; 32],
}

/// The caller must have independently *rehashed the physical private stage*.
/// This function is metadata-only: a remote checkpoint is not accepted as
/// proof, and a prefix hash does not prove the full object or grant a GET.
pub fn plan_resume(
    notice: &AttachmentNoticeV1,
    stage_len: u64,
    physically_checked_prefix_hash: [u8; 32],
) -> Result<(CheckedResume, TransferRequest), AttachmentBridgeError> {
    let size = notice.size();
    let root = notice.root();
    if size == 0 || size > 8 * 1024 * 1024 || root == [0; 32] {
        return Err(AttachmentBridgeError::InvalidRootOrSize);
    }
    if stage_len >= size
        || (stage_len != 0 && stage_len % 65536 != 0)
        || physically_checked_prefix_hash == [0; 32]
    {
        return Err(AttachmentBridgeError::InvalidResume);
    }
    if stage_len == 0 && physically_checked_prefix_hash != *blake3::hash(b"").as_bytes() {
        return Err(AttachmentBridgeError::InvalidResume);
    }
    let range = ByteRange::new(stage_len, size, size)
        .map_err(AttachmentBridgeError::Staya)?;
    let req = TransferRequest::new(
        Manifest { id: ContentId(root), size }, range
    ).map_err(AttachmentBridgeError::Staya)?;
    Ok((CheckedResume { offset: stage_len, verified_prefix: physically_checked_prefix_hash }, req))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aquilum_core::p2p_offer_lab::{
        DecisionAction, OfferedFile, OfferV1, PeerBinding, SCHEMA,
    };
    use aquilum_core::sync::DeviceId;
    use uuid::Uuid;
    fn notice() -> AttachmentNoticeV1 {
        let sender = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(11)),
            endpoint_id: [1; 32]
        };
        let receiver = PeerBinding {
            device_id: DeviceId::from_uuid(Uuid::from_u128(12)),
            endpoint_id: [2; 32]
        };
        let offer = OfferV1 {
            schema: SCHEMA, offer_id: Uuid::from_u128(40), nonce: [3; 16],
            sender, receiver, created_unix: 200, expires_unix: 250,
            files: vec![OfferedFile {
                label: "photo.bin".into(), bytes: 5 * 1024 * 1024,
                blake3_root: [4; 32]
            }]
        };
        let decision = offer.receiver_decision(
            DecisionAction::Accept, vec![0], 201, sender, receiver
        ).unwrap();
        AttachmentNoticeV1::from_accepted(&offer, &decision, sender, receiver, 0, 202).unwrap()
    }

    #[test]
    fn real_staya_range_only_skips_independently_verified_prefix() {
        let n = notice();
        let prefix = *blake3::hash(&vec![0x77; 2 * 1024 * 1024]).as_bytes();
        let (checkpoint, req) = plan_resume(&n, 2 * 1024 * 1024, prefix).unwrap();
        assert_eq!(checkpoint.offset, 2 * 1024 * 1024);
        assert_eq!(req.range.start(), 2 * 1024 * 1024);
        assert_eq!(req.range.end(), 5 * 1024 * 1024);
        assert_eq!(req.manifest.id, ContentId(n.root()));
        assert_eq!(req.manifest.size, n.size());
        let empty = *blake3::hash(b"").as_bytes();
        assert_eq!(plan_resume(&n, 0, empty).unwrap().1.range.start(), 0);
        assert_eq!(plan_resume(&n, 1, prefix), Err(AttachmentBridgeError::InvalidResume));
        assert_eq!(plan_resume(&n, n.size(), prefix), Err(AttachmentBridgeError::InvalidResume));
        assert_eq!(plan_resume(&n, 0, prefix), Err(AttachmentBridgeError::InvalidResume));
        assert_eq!(plan_resume(&n, 2 * 1024 * 1024, [0;32]), Err(AttachmentBridgeError::InvalidResume));
    }
}
