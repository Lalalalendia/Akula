//! Independent, public-API regression tests for untrusted full-frame headers.
//! No File Gate, S3, or authenticated-base authority is implied by decoding.

use akula_text_delta_codec::{
    codec::{decode, encode, TextDeltaError, TextDeltaMode},
    RevisionId,
};
use uuid::Uuid;

fn full_frame() -> (RevisionId, Vec<u8>, Vec<u8>) {
    let revision = RevisionId::from_uuid(Uuid::from_bytes([0x22; 16]));
    let text = "Standalone full frame: Russian text, UTF-8, and a revised chapter.\n"
        .repeat(8).into_bytes();
    let encoded = encode(revision, &text, None).expect("encode standalone full mode");
    assert_eq!(encoded.mode, TextDeltaMode::FullZstd);
    assert_eq!(&encoded.bytes[..8], b"AKTDLT01");
    (revision, text, encoded.bytes)
}

#[test]
fn truncated_full_frame_header_is_always_rejected() {
    let (revision, _, frame) = full_frame();
    // AKTDLT01: magic(8), schema/mode/reserved(4), ids(32), digests(64),
    // three u32 lengths(12) = exactly 120 header bytes.
    const HEADER_LEN: usize = 120;
    assert!(frame.len() > HEADER_LEN);
    for cut in 0..=HEADER_LEN {
        assert_eq!(
            decode(&frame[..cut], revision, None),
            Err(TextDeltaError::InvalidEnvelope),
            "truncated header/payload at offset {cut} must not be accepted",
        );
    }
}

#[test]
fn full_frame_refuses_forged_base_metadata_even_with_external_base_bytes() {
    let (revision, payload, frame) = full_frame();
    let foreign_base = RevisionId::from_uuid(Uuid::from_bytes([0x33; 16]));
    let untrusted = Some((foreign_base, b"untrusted unrelated base".as_slice()));

    // FullZstd is a self-contained representation; an external supplied base
    // must not influence its decoded contents or confer publishing authority.
    assert_eq!(decode(&frame, revision, untrusted).unwrap(), payload);

    // Base revision UUID [12..28], base digest [44..76], and old length
    // [108..112] are strictly zero in a full frame. Never silently ignore a
    // forged base identity or hash in a supposedly independent envelope.
    // Oversized old_len is refused at the budget gate, before the semantic
    // FullZstd zero-base check. Both error paths must remain fail-closed.
    for (offset, expected) in [
        (12usize, TextDeltaError::InvalidEnvelope),
        (27, TextDeltaError::InvalidEnvelope),
        (44, TextDeltaError::InvalidEnvelope),
        (75, TextDeltaError::InvalidEnvelope),
        (108, TextDeltaError::InvalidEnvelope),
        (111, TextDeltaError::OverBudget),
    ] {
        let mut forged = frame.clone();
        forged[offset] ^= 1;
        assert_eq!(
            decode(&forged, revision, untrusted),
            Err(expected),
            "forged FullZstd base field at offset {offset}",
        );
    }
}

#[test]
fn full_frame_rejects_unknown_wire_versions_modes_and_reserved_bits() {
    let (revision, _, frame) = full_frame();
    // Reserved header bits must not be silently accepted as future features.
    // An unknown schema or mode requires explicit negotiation, not guessing.
    for (offset, mask, expected) in [
        (8usize, 1u8, TextDeltaError::Unsupported),
        (9, 0x80, TextDeltaError::Unsupported),
        (10, 1, TextDeltaError::InvalidEnvelope),
        (11, 1, TextDeltaError::InvalidEnvelope),
    ] {
        let mut forged = frame.clone();
        forged[offset] ^= mask;
        assert_eq!(
            decode(&forged, revision, None),
            Err(expected),
            "unrecognized FullZstd header field at offset {offset}",
        );
    }
}

#[test]
fn full_frame_rejects_forged_target_digest_and_payload_length_mismatch() {
    let (revision, _, frame) = full_frame();

    // Integrity covers decoded bytes, not merely a valid zstd stream.
    let mut forged_digest = frame.clone();
    forged_digest[76] ^= 1;
    assert_eq!(decode(&forged_digest, revision, None), Err(TextDeltaError::Integrity));

    // The 32-bit compressed length must match the exact wire frame boundary.
    // Neither appended nor truncated attacker-controlled bytes are accepted.
    let mut appended = frame.clone();
    appended.push(0);
    assert_eq!(decode(&appended, revision, None), Err(TextDeltaError::OverBudget));

    let mut shortened = frame.clone();
    shortened.pop();
    assert_eq!(decode(&shortened, revision, None), Err(TextDeltaError::OverBudget));

    let mut false_length = frame.clone();
    false_length[116] ^= 1;
    assert_eq!(decode(&false_length, revision, None), Err(TextDeltaError::OverBudget));
}
