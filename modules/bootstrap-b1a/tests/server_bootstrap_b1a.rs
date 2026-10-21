//! B1a offline only: no shell, SSH, network, OS service, artifact path or
//! file modifications; test keys are deterministic NOT trusted release keys.
use akula_server_release_b1a::{
    verify_signed_release, ReleaseError, ReleaseFloor, ReleaseManifestV1, SignedRelease,
    TargetPlatform, TrustedReleaseSigner, Version, MAX_NODE_PACKAGE_BYTES,
    MAX_MANIFEST_VALIDITY_SECONDS,
};
use ring::{digest, signature::{self, KeyPair}};
use std::io::{Cursor, Read};
const NOW: u64 = 1_800_000_000;

fn signing_key() -> signature::Ed25519KeyPair {
    // Deliberately public, reproducible TEST key. Never distribute as real
    // Akula release signing authority or ship in an installer trust store.
    signature::Ed25519KeyPair::from_seed_unchecked(&[42u8; 32]).unwrap()
}
fn trusted_anchor() -> TrustedReleaseSigner {
    let key = signing_key();
    TrustedReleaseSigner {
        signer_id: "akula-node-test-release".into(),
        ed25519_public: key.public_key().as_ref().try_into().unwrap(),
    }
}
fn manifest(content: &[u8]) -> ReleaseManifestV1 {
    ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 3, patch: 9 },
        release_sequence: 17,
        package_size: content.len() as u64,
        package_sha256: digest::digest(&digest::SHA256, content)
            .as_ref().try_into().unwrap(),
        not_before_unix: NOW-60,
        expires_unix: NOW + 30*24*60*60,
        signer_id: "akula-node-test-release".into(),
    }
}
fn sign(m: ReleaseManifestV1) -> SignedRelease {
    let signed = signing_key().sign(&m.canonical_signed_bytes());
    SignedRelease { manifest: m, signature: signed.as_ref().try_into().unwrap() }
}
fn open(s: &SignedRelease) -> Result<akula_server_release_b1a::VerifiedRelease, ReleaseError> {
    verify_signed_release(s, &trusted_anchor(), ReleaseFloor::initial(), NOW)
}

#[test]
fn legitimate_signature_and_exact_bytes_produce_only_an_inert_receipt() {
    let package = b"AKULA NODE SYNTHETIC PACKAGE: never executable";
    let s = sign(manifest(package));
    let v = open(&s).unwrap();
    let receipt = v.verify_package_stream(&mut Cursor::new(package)).unwrap();
    assert_eq!(receipt.byte_length, package.len() as u64);
    assert_eq!(receipt.package_sha256, s.manifest.package_sha256);
    assert_eq!(receipt.proposed_floor.sequence, 17);
    assert_eq!(receipt.proposed_floor.version, s.manifest.version);
    assert_eq!(receipt.signed_manifest_fingerprint,
        receipt.proposed_floor.signed_manifest_fingerprint);
    // A release receipt has no installer or writable filesystem method.
}

#[test]
fn forged_metadata_or_signature_refused_even_if_package_hash_matches() {
    let payload = b"strict signed release";
    let signed = sign(manifest(payload));
    let mut forged = signed.clone();
    forged.manifest.version.patch += 1;
    assert_eq!(open(&forged).unwrap_err(), ReleaseError::BadSignature);
    let mut forged = signed.clone();
    forged.manifest.release_sequence += 1;
    assert_eq!(open(&forged).unwrap_err(), ReleaseError::BadSignature);
    let mut forged = signed.clone();
    forged.manifest.package_sha256[0] ^= 1;
    assert_eq!(open(&forged).unwrap_err(), ReleaseError::BadSignature);
    let mut forged = signed.clone();
    forged.manifest.expires_unix -= 1;
    assert_eq!(open(&forged).unwrap_err(), ReleaseError::BadSignature);
    let mut forged = signed.clone();
    forged.signature[0] ^= 1;
    assert_eq!(open(&forged).unwrap_err(), ReleaseError::BadSignature);
}

#[test]
fn candidate_cannot_choose_its_own_trusted_signer() {
    let payload = b"trust anchor separate from SSH";
    let signed = sign(manifest(payload));
    let mut untrusted = trusted_anchor();
    untrusted.signer_id = "different-trusted-release-name".into();
    assert_eq!(verify_signed_release(&signed, &untrusted,
        ReleaseFloor::initial(), NOW).unwrap_err(), ReleaseError::UntrustedSigner);
    let mut untrusted = trusted_anchor();
    untrusted.ed25519_public[0] ^= 1;
    assert_eq!(verify_signed_release(&signed, &untrusted,
        ReleaseFloor::initial(), NOW).unwrap_err(), ReleaseError::BadSignature);
}

#[test]
fn no_downgrade_and_no_same_sequence_equivocation() {
    let payload = b"immutable signed stream";
    let signed = sign(manifest(payload));
    let good = open(&signed).unwrap();
    let floor = good.release_floor_candidate();
    assert!(verify_signed_release(&signed, &trusted_anchor(), floor, NOW).is_ok(),
        "idempotent retry of same exactly signed release may be admitted");
    let mut older = signed.manifest.clone();
    older.release_sequence = 16;
    assert_eq!(verify_signed_release(&sign(older), &trusted_anchor(), floor, NOW)
        .unwrap_err(), ReleaseError::RollbackRefused);
    let mut equivocated = signed.manifest.clone();
    equivocated.package_sha256[2] ^= 1;
    assert_eq!(verify_signed_release(&sign(equivocated), &trusted_anchor(), floor, NOW)
        .unwrap_err(), ReleaseError::RollbackRefused,
        "a different signed artifact at the same sequence is forbidden");
    let mut version_down = signed.manifest.clone();
    version_down.release_sequence = 18;
    version_down.version.patch = 8;
    assert_eq!(verify_signed_release(&sign(version_down), &trusted_anchor(), floor, NOW)
        .unwrap_err(), ReleaseError::RollbackRefused,
        "a higher sequence must not override a lower semantic version");
    let mut upgraded = signed.manifest.clone();
    upgraded.release_sequence += 1;
    upgraded.version.patch += 1;
    assert!(verify_signed_release(&sign(upgraded), &trusted_anchor(), floor, NOW).is_ok());
}

#[test]
fn expiration_and_not_before_are_rejected_before_content_access() {
    let source = b"no clock bypass";
    let signed = sign(manifest(source));
    assert_eq!(verify_signed_release(&signed, &trusted_anchor(),
        ReleaseFloor::initial(), NOW-61).unwrap_err(), ReleaseError::NotYetValid);
    assert_eq!(verify_signed_release(&signed, &trusted_anchor(),
        ReleaseFloor::initial(), signed.manifest.expires_unix)
        .unwrap_err(), ReleaseError::Expired);

    let mut oversized_validity = signed.manifest.clone();
    oversized_validity.expires_unix =
        oversized_validity.not_before_unix + MAX_MANIFEST_VALIDITY_SECONDS + 1;
    assert_eq!(open(&sign(oversized_validity)).unwrap_err(), ReleaseError::InvalidMetadata);
}

#[test]
fn invalid_manifest_metadata_denied_even_with_valid_signature() {
    let source = b"test-only";
    let m = manifest(source);
    let bad = [
        { let mut v=m.clone(); v.release_sequence=0; v },
        { let mut v=m.clone(); v.package_size=0; v },
        { let mut v=m.clone(); v.package_size=MAX_NODE_PACKAGE_BYTES+1; v },
        { let mut v=m.clone(); v.package_sha256=[0;32]; v },
        { let mut v=m.clone(); v.signer_id="../outside".into(); v },
        { let mut v=m.clone(); v.signer_id=" ".repeat(300); v },
        { let mut v=m.clone(); v.expires_unix=v.not_before_unix; v },
    ];
    for candidate in bad {
        assert_eq!(open(&sign(candidate)).unwrap_err(), ReleaseError::InvalidMetadata);
    }
}

#[test]
fn tampered_truncated_or_appended_binary_refused() {
    let binary = b"twelve bytes";
    let signed = sign(manifest(binary));
    let verified = open(&signed).unwrap();
    let mut flipped = binary.to_vec();
    flipped[0] ^= 1;
    assert_eq!(verified.verify_package_stream(&mut Cursor::new(flipped))
        .unwrap_err(), ReleaseError::PackageDigestMismatch);
    assert_eq!(verified.verify_package_stream(&mut Cursor::new(&binary[..3]))
        .unwrap_err(), ReleaseError::SizeMismatch);
    let mut extended = binary.to_vec();
    extended.push(0);
    assert_eq!(verified.verify_package_stream(&mut Cursor::new(extended))
        .unwrap_err(), ReleaseError::SizeMismatch);
}

struct BrokenReader;
impl Read for BrokenReader {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(std::io::ErrorKind::Other, "test-only read failure"))
    }
}
#[test]
fn io_error_is_not_a_successful_receipt() {
    let signed = sign(manifest(b"transport errors are refusals"));
    assert_eq!(open(&signed).unwrap().verify_package_stream(&mut BrokenReader)
        .unwrap_err(), ReleaseError::StreamIo);
}
