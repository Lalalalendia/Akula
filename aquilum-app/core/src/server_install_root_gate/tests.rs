use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};
use uuid::Uuid;

const NOW: u64 = 1_800_000_000;
const HOLD: RootHelperReadiness = RootHelperReadiness::NeedsAdminAuthorization;

fn signed_fixture() -> (ReviewEcho, RootHelperPins, RootHelperEvidence) {
    let key = Ed25519KeyPair::from_seed_unchecked(&[0x91; 32]).unwrap();
    let review = ReviewEcho {
        install_id: Uuid::from_bytes([0x11; 16]),
        plan_hash: [0x22; 32],
        ssh_host_key_sha256: [0x33; 32],
        package_sha256: [0x44; 32],
        review_nonce: [0x55; 16],
    };
    let pins = RootHelperPins {
        attestor_ed25519: key.public_key().as_ref().try_into().unwrap(),
        helper_sha256: [0x66; 32],
        owner_ed25519: [0x77; 32],
        root_policy_sha256: [0x88; 32],
    };
    let mut proof = RootHelperEvidence {
        install_id: *review.install_id.as_bytes(),
        plan_blake3: review.plan_hash,
        ssh_host_sha256: review.ssh_host_key_sha256,
        package_sha256: review.package_sha256,
        review_nonce: review.review_nonce,
        helper_sha256: pins.helper_sha256,
        owner_ed25519: pins.owner_ed25519,
        root_policy_sha256: pins.root_policy_sha256,
        issued_at_unix: NOW - 5,
        expires_at_unix: NOW + 45,
        signature: [0; 64],
    };
    proof.signature = key.sign(&proof.canonical_signed_bytes()).as_ref()
        .try_into().unwrap();
    (review, pins, proof)
}

#[test]
fn only_pre_enrolled_signed_presence_is_observed() {
    let (review, pins, proof) = signed_fixture();
    assert_eq!(assess_pre_enrolled_helper(&review, &pins, None, NOW), HOLD);
    assert_eq!(
        assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW),
        RootHelperReadiness::AuthenticatedPresenceOnly,
    );
}

#[test]
fn mismatched_scope_and_unpinned_keys_hold() {
    for variant in 0..11 {
        let (mut review, mut pins, mut proof) = signed_fixture();
        match variant {
            0 => proof.install_id[0] ^= 1,
            1 => proof.plan_blake3[0] ^= 1,
            2 => proof.ssh_host_sha256[0] ^= 1,
            3 => proof.package_sha256[0] ^= 1,
            4 => proof.review_nonce[0] ^= 1,
            5 => proof.helper_sha256[0] ^= 1,
            6 => proof.owner_ed25519[0] ^= 1,
            7 => proof.root_policy_sha256[0] ^= 1,
            8 => proof.signature[0] ^= 1,
            9 => pins.attestor_ed25519 = [0; 32],
            10 => review.review_nonce[0] ^= 1,
            _ => unreachable!(),
        }
        if variant <= 7 {
            // A root signer can sign a different *valid* candidate;
            // that must still be refused against independently pinned scope.
            let signer = Ed25519KeyPair::from_seed_unchecked(&[0x91; 32]).unwrap();
            proof.signature = signer.sign(&proof.canonical_signed_bytes())
                .as_ref().try_into().unwrap();
        }
        assert_eq!(
            assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW),
            HOLD,
            "variant {variant}",
        );
    }
}

#[test]
fn expired_future_or_unbounded_evidence_holds() {
    let (review, pins, proof) = signed_fixture();
    assert_eq!(
        assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW + 45),
        HOLD,
    );
    assert_eq!(
        assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW - 6),
        HOLD,
    );
    let mut too_long = proof;
    too_long.expires_at_unix = NOW + MAX_TTL_SECONDS;
    assert_eq!(
        assess_pre_enrolled_helper(&review, &pins, Some(&too_long), NOW),
        HOLD,
    );
}
