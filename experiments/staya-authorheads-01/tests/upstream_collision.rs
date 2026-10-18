//! Real upstream iroh-docs v0.101.0 AuthorHeads regression probe.
//! "same timestamp" is intentionally RED until upstream behavior is fixed.
//! Run it separately; do not make ordinary Akula builds fail.

use iroh_docs::{AuthorHeads, AuthorId};

#[test]
fn distinct_timestamps_are_preserved_control_green() {
    let mut heads = AuthorHeads::default();
    heads.insert(AuthorId::from(&[1u8; 32]), 1_700_000_000);
    heads.insert(AuthorId::from(&[2u8; 32]), 1_700_000_001);
    assert_eq!(heads.len(), 2, "control input should contain both authors");
    let wire = heads.encode(None).expect("encode");
    let roundtrip = AuthorHeads::decode(&wire).expect("decode");
    assert_eq!(roundtrip.len(), 2, "control: distinct timestamp heads should survive");
}

#[test]
fn equal_timestamps_for_two_authors_must_both_survive_expected_red() {
    let mut heads = AuthorHeads::default();
    let first = AuthorId::from(&[3u8; 32]);
    let second = AuthorId::from(&[4u8; 32]);
    heads.insert(first, 1_700_000_000);
    heads.insert(second, 1_700_000_000);
    assert_eq!(heads.len(), 2, "input contains both authors");
    let wire = heads.encode(None).expect("encode without size truncation");
    let roundtrip = AuthorHeads::decode(&wire).expect("decode");
    assert_eq!(
        roundtrip.len(), 2,
        "STAYA_EXPECTED_HEADS_COLLISION_LEN2: encode(None) dropped one of two authors with the same timestamp"
    );
    assert_eq!(roundtrip.get(&first), Some(1_700_000_000));
    assert_eq!(roundtrip.get(&second), Some(1_700_000_000));
}
