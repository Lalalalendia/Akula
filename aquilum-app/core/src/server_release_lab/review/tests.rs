use super::*;
use crate::server_bootstrap_lab::{
    preview, B0Error, B0Input, PreviewTransport, ReadOnlyHostFacts,
};
use crate::server_release_lab::{
    ReleaseManifestV1, SignedRelease, TargetPlatform, TrustedReleaseSigner,
    Version,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use ring::digest::{digest, SHA256};
use std::io::Cursor;

const NOW: u64 = 1_600_000_000;
const PACKAGE: &[u8] = b"\x7fELF Akula B1-E2E-02 fake signed package fixture";

struct VerifiedKexFixture {
    wire: Vec<u8>,
}
impl PreviewTransport for VerifiedKexFixture {
    fn handshake_public_key(
        &mut self, _: &ServerTarget,
    ) -> Result<Vec<u8>, B0Error> {
        Ok(self.wire.clone())
    }

    fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
        Ok(ReadOnlyHostFacts {
            os: SupportedOs::Ubuntu,
            arch: MachineArch::X86_64,
            has_systemd: true,
            has_sudo_rights: true,
            cpu_count: 4,
            ram_mib: 4096,
            free_disk_mib: 2048,
        })
    }
}

fn fixture() -> (DryRunPlan, SignedRelease, TrustedReleaseSigner, Vec<u8>) {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[19u8; 32]).unwrap();
    let package = PACKAGE.to_vec();
    let manifest = ReleaseManifestV1 {
        platform: TargetPlatform::LinuxX86_64,
        version: Version { major: 0, minor: 3, patch: 13 },
        release_sequence: 9,
        package_size: package.len() as u64,
        package_sha256: digest(&SHA256, &package).as_ref().try_into().unwrap(),
        not_before_unix: NOW - 1,
        expires_unix: NOW + 3600,
        signer_id: "akula-test-fixture".into(),
    };
    let signature = pair.sign(&manifest.canonical_signed_bytes());
    let signed = SignedRelease {
        manifest: manifest.clone(),
        signature: signature.as_ref().try_into().unwrap(),
    };
    let trusted = TrustedReleaseSigner {
        signer_id: manifest.signer_id.clone(),
        ed25519_public: pair.public_key().as_ref().try_into().unwrap(),
    };
    let wire = b"SSH-server-public-key-fixture-never-real".to_vec();
    let target = ServerTarget {
        host: "vps.example.org".into(),
        ssh_port: 22,
        ssh_user: "ubuntu".into(),
        trusted_key_sha256: digest(&SHA256, &wire).as_ref().try_into().unwrap(),
    };
    let input = B0Input {
        install_id: Uuid::new_v4(),
        target,
        package: PackagePin {
            version: "0.3.13".into(),
            package_sha256: manifest.package_sha256,
            trusted_signer_id: manifest.signer_id,
        },
    };
    let plan = preview(
        &input, &mut VerifiedKexFixture { wire }, || false,
    ).expect("read-only pinned KEX fixture must generate a B0 plan");
    assert!(plan.verify_unchanged());
    (plan, signed, trusted, package)
}

fn prepare(
    plan: DryRunPlan, signed: &SignedRelease,
    trusted: &TrustedReleaseSigner, artifact: &[u8],
) -> Result<PreparedReview, ReviewError> {
    prepare_signed_review(
        plan, signed, trusted, ReleaseFloor::initial(),
        &mut Cursor::new(artifact), NOW,
    )
}

#[test]
fn signed_package_and_exact_review_click_are_single_use() {
    let (plan, signed, trusted, artifact) = fixture();
    let mut review = prepare(plan, &signed, &trusted, &artifact).unwrap();
    let echo = review.expected_echo();
    let target = review.shown_plan().target.clone();
    let package = review.shown_plan().package.clone();
    let consent = review.consume_local_click(
        &echo, &target, &package, NOW + 10,
    ).expect("local click");
    let scope = consent.scope();
    assert_eq!(scope.0, *echo.install_id.as_bytes());
    assert_eq!(scope.1, echo.plan_hash);
    assert_eq!(scope.2, echo.ssh_host_key_sha256);
    assert_eq!(scope.3, echo.package_sha256);
    assert_ne!(scope.4, [0; 32]);
    assert_eq!(consent.nonce(), echo.review_nonce);
    assert_eq!(
        review.consume_local_click(&echo, &target, &package, NOW + 11),
        Err(ReviewError::AlreadyUsed),
    );
}

#[test]
fn changed_host_package_nonce_or_plan_digest_consumes_review() {
    for variant in 0..6 {
        let (plan, signed, trusted, artifact) = fixture();
        let mut review = prepare(plan, &signed, &trusted, &artifact).unwrap();
        let mut echo = review.expected_echo();
        let mut target = review.shown_plan().target.clone();
        let mut package = review.shown_plan().package.clone();
        match variant {
            0 => target.host = "another.example.org".into(),
            1 => target.trusted_key_sha256[0] ^= 0x80,
            2 => target.ssh_user = "other".into(),
            3 => package.package_sha256[0] ^= 1,
            4 => echo.plan_hash[0] ^= 1,
            5 => echo.review_nonce[0] ^= 1,
            _ => unreachable!(),
        }
        assert_eq!(
            review.consume_local_click(&echo, &target, &package, NOW + 2),
            Err(ReviewError::StaleOrChanged),
            "variant {variant}",
        );
        assert_eq!(
            review.consume_local_click(&echo, &target, &package, NOW + 3),
            Err(ReviewError::AlreadyUsed),
        );
    }
}

#[test]
fn wrong_signature_manifest_or_artifact_never_builds_review() {
    let (plan, signed, trusted, package) = fixture();
    let mut bad_sign = signed.clone();
    bad_sign.signature[0] ^= 1;
    assert!(prepare(plan.clone(), &bad_sign, &trusted, &package).is_err());
    let mut bad_data = package.clone();
    bad_data[4] ^= 1;
    assert!(prepare(plan.clone(), &signed, &trusted, &bad_data).is_err());
    let mut extra = package.clone();
    extra.push(0);
    assert!(prepare(plan.clone(), &signed, &trusted, &extra).is_err());
    let mut bad_signer = trusted.clone();
    bad_signer.ed25519_public[0] ^= 1;
    assert!(prepare(plan.clone(), &signed, &bad_signer, &package).is_err());
    let mut other_package = plan.clone();
    other_package.package.version = "0.3.14".into();
    other_package.plan_blake3 = other_package.computed_digest();
    assert!(matches!(
        prepare(other_package, &signed, &trusted, &package),
        Err(ReviewError::InvalidPackage),
    ));
    let mut wrong_floor = ReleaseFloor::initial();
    wrong_floor.sequence = 10;
    wrong_floor.version = Version { major: 0, minor: 3, patch: 14 };
    assert!(prepare_signed_review(
        plan, &signed, &trusted, wrong_floor,
        &mut Cursor::new(package), NOW,
    ).is_err());
}

#[test]
fn changed_preview_and_unsupported_host_are_refused_even_if_rehashed() {
    let (plan, signed, trusted, artifact) = fixture();
    let mut modified = plan.clone();
    modified.proposed_changes.clear();
    modified.plan_blake3 = modified.computed_digest();
    assert!(matches!(
        prepare(modified, &signed, &trusted, &artifact),
        Err(ReviewError::InvalidPreview),
    ));
    let mut wrong_host = plan.clone();
    wrong_host.inspected_host.has_systemd = false;
    wrong_host.plan_blake3 = wrong_host.computed_digest();
    assert!(matches!(
        prepare(wrong_host, &signed, &trusted, &artifact),
        Err(ReviewError::InvalidPreview),
    ));
    let mut invalid_digest = plan;
    invalid_digest.plan_blake3 = "g".repeat(64);
    assert!(matches!(
        prepare(invalid_digest, &signed, &trusted, &artifact),
        Err(ReviewError::InvalidPreview),
    ));
}

#[test]
fn cancel_expiry_and_clock_rollback_fail_closed() {
    let (plan, signed, trusted, artifact) = fixture();
    let mut cancelled = prepare(plan.clone(), &signed, &trusted, &artifact).unwrap();
    let echo = cancelled.expected_echo();
    let target = cancelled.shown_plan().target.clone();
    let package = cancelled.shown_plan().package.clone();
    cancelled.cancel();
    assert_eq!(
        cancelled.consume_local_click(&echo, &target, &package, NOW + 1),
        Err(ReviewError::AlreadyUsed),
    );
    let mut expired = prepare(plan.clone(), &signed, &trusted, &artifact).unwrap();
    let expiry = expired.expires_at_unix();
    let expired_echo = expired.expected_echo();
    assert_eq!(
        expired.consume_local_click(&expired_echo, &target, &package, expiry),
        Err(ReviewError::Expired),
    );
    let mut rollback = prepare(plan, &signed, &trusted, &artifact).unwrap();
    let rollback_echo = rollback.expected_echo();
    assert_eq!(
        rollback.consume_local_click(&rollback_echo, &target, &package, NOW - 1),
        Err(ReviewError::InvalidClock),
    );
    assert_eq!(
        rollback.consume_local_click(&rollback_echo, &target, &package, NOW + 1),
        Err(ReviewError::AlreadyUsed),
    );
}
