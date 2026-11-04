//! Typed adapter from Core RevisionId to standalone wire-codec RevisionId.
//! History trust, File Gate authority and S3 V2 admission stay in Core.
use super::RevisionId;
use akula_text_delta_codec::RevisionId as CodecRevisionId;
#[allow(unused_imports)]
pub(crate) use akula_text_delta_codec::codec::{
    EncodedTextDelta, TextDeltaError, TextDeltaMode, MAX_TEXT_BYTES,
};

pub(crate) fn encode(
    target_id: RevisionId,
    target: &[u8],
    old: Option<(RevisionId, &[u8])>,
) -> Result<EncodedTextDelta, TextDeltaError> {
    akula_text_delta_codec::codec::encode(
        CodecRevisionId::from_uuid(target_id.as_uuid()),
        target,
        old.map(|(id, bytes)| (CodecRevisionId::from_uuid(id.as_uuid()), bytes)),
    )
}

pub(crate) fn decode(
    bytes: &[u8],
    expected_revision: RevisionId,
    old: Option<(RevisionId, &[u8])>,
) -> Result<Vec<u8>, TextDeltaError> {
    akula_text_delta_codec::codec::decode(
        bytes,
        CodecRevisionId::from_uuid(expected_revision.as_uuid()),
        old.map(|(id, data)| (CodecRevisionId::from_uuid(id.as_uuid()), data)),
    )
}
