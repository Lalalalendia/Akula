use super::*;
use crate::server_release_lab::{ReleaseManifestV1, TargetPlatform, Version};
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};

const NOW: u64 = 1_800_000_000;

fn fixture() -> (SignedRelease, TrustedReleaseSigner, Vec<u8>) {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[0x37; 32]).unwrap();
    let mut helper = vec![0u8; 128];
    helper[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    helper[18..20].copy_from_slice(b"\x3e\x00");
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 3, patch: 13 },
        release_sequence: 1,
        package_size: helper.len() as u64,
        package_sha256: digest(&SHA256, &helper).as_ref().try_into().unwrap(),
        not_before_unix: NOW - 100,
        expires_unix: NOW + 100,
        signer_id: BOOTSTRAP_SIGNER.into(),
    };
    let signature = pair.sign(&manifest.canonical_signed_bytes());
    (
        SignedRelease {
            manifest,
            signature: signature.as_ref().try_into().unwrap(),
        },
        TrustedReleaseSigner {
            signer_id: BOOTSTRAP_SIGNER.into(),
            ed25519_public: pair.public_key().as_ref().try_into().unwrap(),
        },
        helper,
    )
}

#[test]
fn approved_bytes_emit_only_fixed_signed_root_stage_data() {
    let (signed, trusted, helper) = fixture();
    let data = prepare_root_stage_bundle(
        &signed, &trusted, ReleaseFloor::initial(), &helper, NOW,
    )
    .unwrap();
    assert!(data.frame().starts_with(FRAME_DOMAIN));
    let start = FRAME_DOMAIN.len();
    let size = u16::from_be_bytes(data.frame()[start..start + 2].try_into().unwrap()) as usize;
    assert_eq!(size, signed.manifest.canonical_signed_bytes().len());
    assert_eq!(&data.frame()[start + 2 + size..start + 2 + size + 64], &signed.signature[..]);
    assert_eq!(&data.frame()[start + 2 + size + 64..], helper);
    assert!(!data.fixed_root_source().contains(PIN_PLACEHOLDER));
    assert_eq!(data.package_sha256(), signed.manifest.package_sha256);
    assert!(data.fixed_root_source().contains("PINNED_BOOTSTRAP_ED25519_HEX = \""));
}

#[test]
fn altered_helper_key_signature_or_role_cannot_create_frame() {
    let (signed, trusted, helper) = fixture();
    let mut changed_helper = helper.clone();
    changed_helper[100] ^= 1;
    assert!(prepare_root_stage_bundle(
        &signed, &trusted, ReleaseFloor::initial(), &changed_helper, NOW,
    )
    .is_err());
    let mut changed_signature = signed.clone();
    changed_signature.signature[0] ^= 1;
    assert!(prepare_root_stage_bundle(
        &changed_signature, &trusted, ReleaseFloor::initial(), &helper, NOW,
    )
    .is_err());
    let mut wrong_key = trusted.clone();
    wrong_key.ed25519_public[0] ^= 1;
    assert!(prepare_root_stage_bundle(
        &signed, &wrong_key, ReleaseFloor::initial(), &helper, NOW,
    )
    .is_err());
    wrong_key.signer_id = "ordinary-node".into();
    assert_eq!(
        prepare_root_stage_bundle(&signed, &wrong_key, ReleaseFloor::initial(), &helper, NOW)
            .err(),
        Some(RootStageError::UntrustedBootstrapSigner),
    );
}
