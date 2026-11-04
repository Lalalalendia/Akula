//! Standalone, pure bounded Text Delta wire representation.
//!
//! This crate owns production codec logic, but cannot establish history trust,
//! grant File Gate rights, transport S3 packets, or publish any baseline.
use uuid::Uuid;

/// Intentionally matches the Core RevisionId wire identity without depending
/// on aquilum-core or importing any of its trust or storage capabilities.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RevisionId(Uuid);

impl RevisionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }
    pub fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RevisionId {
    fn default() -> Self {
        Self::new()
    }
}

pub mod codec;

#[cfg(test)]
mod parity_boundary_tests {
    use super::*;

    #[test]
    fn immutable_revision_identity_and_frame_contract() {
        let parent = RevisionId::from_uuid(Uuid::from_bytes([0x11; 16]));
        let target = RevisionId::from_uuid(Uuid::from_bytes([0x22; 16]));
        let original = "Public example: old markdown data.\n".repeat(20);
        let revised = format!("{original}\nUpdated data.\n");
        let frame = codec::encode(target, revised.as_bytes(), Some((parent, original.as_bytes())))
            .expect("pure codec encoding");
        assert_eq!(&frame.bytes[..8], b"AKTDLT01");
        assert_eq!(&frame.bytes[28..44], target.as_uuid().as_bytes());
        let decoded = codec::decode(&frame.bytes, target, Some((parent, original.as_bytes())))
            .expect("pure codec decoding");
        assert_eq!(decoded, revised.as_bytes());
        assert_eq!(codec::decode(&frame.bytes, RevisionId::new(), Some((parent, original.as_bytes()))),
            Err(codec::TextDeltaError::WrongRevision));
    }
}
