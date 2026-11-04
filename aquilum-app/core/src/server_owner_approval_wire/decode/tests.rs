use super::*;
use ring::signature::{Ed25519KeyPair, KeyPair};

const NOW: u64 = 1_800_000_000;

fn candidate() -> OwnerApprovalWireV1 {
    OwnerApprovalWireV1 {
        approver_id: "owner-e2e-07".into(),
        install_id: [0x11; 16],
        plan_digest: [0x22; 32],
        ssh_host_sha256: [0x33; 32],
        manifest_fingerprint: [0x44; 32],
        previous: OwnerFloorWire {
            generation: 0,
            sequence: 0,
            version: [0; 3],
            manifest_fingerprint: [0; 32],
            chain_tip: [0; 32],
        },
        proposed: OwnerFloorWire {
            generation: 1,
            sequence: 7,
            version: [0, 3, 13],
            manifest_fingerprint: [0x44; 32],
            chain_tip: [0x55; 32],
        },
        staged_package_sha256: [0x66; 32],
        staged_package_size: 64_000,
        expected_witness_revision: 0,
        approval_nonce: [0x77; 16],
        issued_at_unix: NOW - 5,
        expires_at_unix: NOW + 120,
    }
}

#[test]
fn canonical_portable_frame_roundtrip_uses_independently_pinned_owner() {
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x92; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let claim = candidate();
    let canonical = claim.canonical_bytes();
    assert!(canonical.len() <= MAX_CANONICAL_BYTES);
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&canonical),
        Ok(claim.clone())
    );
    let signature: [u8; 64] = signer.sign(&canonical).as_ref().try_into().unwrap();
    let mut bytes = canonical.clone();
    bytes.extend_from_slice(&signature);
    let frame = SignedOwnerWireFrame::decode(&bytes).unwrap();
    assert_eq!(frame.canonical_frame(), bytes);
    assert_eq!(
        frame.verify_pinned_owner("owner-e2e-07", &public, NOW),
        Ok(())
    );
    assert_eq!(
        frame.verify_pinned_owner("different-owner", &public, NOW),
        Err(WireError::UntrustedOwner)
    );
    assert_eq!(
        frame.verify_pinned_owner("owner-e2e-07", &[0; 32], NOW),
        Err(WireError::UntrustedOwner)
    );
    assert_eq!(
        frame.verify_pinned_owner("owner-e2e-07", &public, NOW + 120),
        Err(WireError::Expired)
    );
}

#[test]
fn truncation_extra_bytes_and_wrong_encoding_never_parse() {
    let original = candidate().canonical_bytes();
    for end in 0..original.len() {
        assert_eq!(
            OwnerApprovalWireV1::decode_canonical_bytes(&original[..end]),
            Err(WireError::MalformedClaim),
            "truncated at {end}"
        );
    }
    let mut trailing = original.clone();
    trailing.push(0);
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&trailing),
        Err(WireError::MalformedClaim)
    );
    let mut wrong_domain = original.clone();
    wrong_domain[0] ^= 1;
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&wrong_domain),
        Err(WireError::MalformedClaim)
    );
    let mut wrong_version = original.clone();
    wrong_version[DOMAIN.len()] = 2;
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&wrong_version),
        Err(WireError::MalformedClaim)
    );
    let mut wrong_length = original.clone();
    wrong_length[DOMAIN.len() + 1] = 65;
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&wrong_length),
        Err(WireError::MalformedClaim)
    );
    let mut invalid_utf8 = original.clone();
    invalid_utf8[DOMAIN.len() + 2] = 0xff;
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&invalid_utf8),
        Err(WireError::MalformedClaim)
    );
    let mut bogus_signature = original.clone();
    bogus_signature.extend_from_slice(&[0; 64]);
    let mut extra = bogus_signature.clone();
    extra.push(0);
    assert_eq!(
        SignedOwnerWireFrame::decode(&extra),
        Err(WireError::MalformedClaim)
    );
    assert_eq!(
        SignedOwnerWireFrame::decode(&original),
        Err(WireError::MalformedClaim)
    );
}

#[test]
fn changed_scopes_require_a_fresh_signature() {
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x93; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let claim = candidate();
    let signature: [u8; 64] = signer
        .sign(&claim.canonical_bytes())
        .as_ref()
        .try_into()
        .unwrap();
    for variant in 0..9 {
        let mut changed = claim.clone();
        match variant {
            0 => changed.install_id[0] ^= 1,
            1 => changed.plan_digest[0] ^= 1,
            2 => changed.ssh_host_sha256[0] ^= 1,
            3 => changed.manifest_fingerprint[0] ^= 1,
            4 => changed.proposed.sequence += 1,
            5 => changed.proposed.chain_tip[0] ^= 1,
            6 => changed.staged_package_sha256[0] ^= 1,
            7 => changed.approval_nonce[0] ^= 1,
            8 => changed.expected_witness_revision += 1,
            _ => unreachable!(),
        }
        let mut wire = changed.canonical_bytes();
        wire.extend_from_slice(&signature);
        let decoded = SignedOwnerWireFrame::decode(&wire).unwrap();
        assert_eq!(
            decoded.verify_pinned_owner(&claim.approver_id, &public, NOW),
            Err(WireError::BadSignature),
            "changed field {variant}"
        );
    }
}

#[test]
fn malformed_claim_and_invalid_clock_refuse() {
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x94; 32]).unwrap();
    let public: [u8; 32] = signer.public_key().as_ref().try_into().unwrap();
    let mut broken = candidate();
    broken.plan_digest = [0; 32];
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&broken.canonical_bytes()),
        Err(WireError::MalformedClaim)
    );
    broken = candidate();
    broken.expires_at_unix = broken.issued_at_unix;
    assert_eq!(
        OwnerApprovalWireV1::decode_canonical_bytes(&broken.canonical_bytes()),
        Err(WireError::MalformedClaim)
    );
    let c = candidate();
    let mut signed = c.canonical_bytes();
    let signature = signer.sign(&signed);
    signed.extend_from_slice(signature.as_ref());
    let decoded = SignedOwnerWireFrame::decode(&signed).unwrap();
    assert_eq!(
        decoded.verify_pinned_owner(&c.approver_id, &public, c.issued_at_unix - 1),
        Err(WireError::NotYetValid)
    );
}

#[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1b-approval-lab"))]
#[test]
fn decoded_frame_converts_to_exact_existing_linux_b1b1b_claim() {
    let signer = Ed25519KeyPair::from_seed_unchecked(&[0x95; 32]).unwrap();
    let candidate = candidate();
    let mut bytes = candidate.canonical_bytes();
    let signature: [u8; 64] = signer.sign(&bytes).as_ref().try_into().unwrap();
    bytes.extend_from_slice(&signature);
    let restored = SignedOwnerWireFrame::decode(&bytes)
        .unwrap()
        .into_inert_linux_claim();
    assert_eq!(
        restored.claim.canonical_bytes(),
        candidate.canonical_bytes()
    );
    assert_eq!(restored.signature, signature);
}
