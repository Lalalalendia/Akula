use super::*;
use crate::server_bootstrap_lab::{
    preview, preview_node_v2, B0Error, B0Input, PreviewTransport, ReadOnlyHostFacts,
};
use crate::server_release_lab::{
    ReleaseManifestV1, SignedRelease, TargetPlatform, TrustedReleaseSigner, Version,
};
use ring::digest::{digest, SHA256};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::io::Cursor;

const NOW: u64 = 1_600_000_000;
const PACKAGE: &[u8] = b"\x7fELF Akula B1-E2E-02 fake signed package fixture";

struct VerifiedKexFixture {
    wire: Vec<u8>,
}
impl PreviewTransport for VerifiedKexFixture {
    fn handshake_public_key(&mut self, _: &ServerTarget) -> Result<Vec<u8>, B0Error> {
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
        version: Version {
            major: 0,
            minor: 3,
            patch: 13,
        },
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
    let plan = preview_node_v2(&input, &mut VerifiedKexFixture { wire }, || false)
        .expect("read-only pinned KEX fixture must generate a B0 plan");
    assert!(plan.verify_unchanged());
    (plan, signed, trusted, package)
}

fn prepare(
    plan: DryRunPlan,
    signed: &SignedRelease,
    trusted: &TrustedReleaseSigner,
    artifact: &[u8],
) -> Result<PreparedReview, ReviewError> {
    prepare_signed_review(
        plan,
        signed,
        trusted,
        ReleaseFloor::initial(),
        &mut Cursor::new(artifact),
        NOW,
    )
}

#[test]
fn signed_package_and_exact_review_click_are_single_use() {
    let (plan, signed, trusted, artifact) = fixture();
    let mut review = prepare(plan, &signed, &trusted, &artifact).unwrap();
    let echo = review.expected_echo();
    let target = review.shown_plan().target.clone();
    let package = review.shown_plan().package.clone();
    let consent = review
        .consume_local_click(&echo, &target, &package, NOW + 10)
        .expect("local click");
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
    wrong_floor.version = Version {
        major: 0,
        minor: 3,
        patch: 14,
    };
    assert!(prepare_signed_review(
        plan,
        &signed,
        &trusted,
        wrong_floor,
        &mut Cursor::new(package),
        NOW,
    )
    .is_err());
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
fn clean_vps_readonly_preview_without_sudo_cannot_approve_install() {
    let (mut plan, signed, trusted, artifact) = fixture();
    plan.inspected_host.has_sudo_rights = false;
    plan.plan_blake3 = plan.computed_digest();
    assert!(plan.verify_unchanged());
    assert!(matches!(
        prepare(plan, &signed, &trusted, &artifact),
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

#[test]
fn local_review_cannot_approve_changed_owner_wire_scope() {
    use crate::server_owner_approval_wire::{OwnerApprovalWireV1, OwnerFloorWire};
    let (plan, signed, trusted, artifact) = fixture();
    let mut review = prepare(plan, &signed, &trusted, &artifact).unwrap();
    let echo = review.expected_echo();
    let target = review.shown_plan().target.clone();
    let package = review.shown_plan().package.clone();
    let consent = review
        .consume_local_click(&echo, &target, &package, NOW + 1)
        .unwrap();
    let (install_id, plan_digest, host_key, package_digest, manifest) = consent.scope();
    let mut intent = OwnerApprovalWireV1 {
        approver_id: "independent-test-owner".into(),
        install_id,
        plan_digest,
        ssh_host_sha256: host_key,
        manifest_fingerprint: manifest,
        previous: OwnerFloorWire {
            generation: 0,
            sequence: 0,
            version: [0; 3],
            manifest_fingerprint: [0; 32],
            chain_tip: [0; 32],
        },
        proposed: OwnerFloorWire {
            generation: 1,
            sequence: 9,
            version: [0, 3, 13],
            manifest_fingerprint: manifest,
            chain_tip: [0x77; 32],
        },
        staged_package_sha256: package_digest,
        staged_package_size: artifact.len() as u64,
        expected_witness_revision: 0,
        approval_nonce: consent.nonce(),
        issued_at_unix: NOW,
        expires_at_unix: NOW + 120,
    };
    assert!(consent.matches_owner_claim(&intent));
    intent.approval_nonce[0] ^= 1;
    assert!(!consent.matches_owner_claim(&intent));
    intent.approval_nonce[0] ^= 1;
    intent.previous.generation = 5;
    // The independently pinned previous CAS head MUST also be checked by
    // the Linux root witness; client local review does not authenticate it.
    assert!(consent.matches_owner_claim(&intent));
    intent.ssh_host_sha256[0] ^= 1;
    assert!(!consent.matches_owner_claim(&intent));
    intent.ssh_host_sha256[0] ^= 1;
    intent.manifest_fingerprint[0] ^= 1;
    assert!(!consent.matches_owner_claim(&intent));
    intent.manifest_fingerprint[0] ^= 1;
    intent.proposed.manifest_fingerprint[0] ^= 1;
    assert!(!consent.matches_owner_claim(&intent));
}

#[cfg(all(target_os = "windows", feature = "server-owner-windows-signer"))]
#[test]
fn windows_owner_key_signs_only_the_reviewed_claim() {
    use crate::server_owner_approval_wire::{OwnerApprovalWireV1, OwnerFloorWire, WireError};
    use crate::server_owner_windows::{OwnerIdentity, OwnerIdentityError};
    let (plan, signed, trusted, artifact) = fixture();
    let mut review = prepare(plan, &signed, &trusted, &artifact).unwrap();
    let echo = review.expected_echo();
    let target = review.shown_plan().target.clone();
    let package = review.shown_plan().package.clone();
    let consent = review
        .consume_local_click(&echo, &target, &package, NOW + 1)
        .unwrap();
    let owner = OwnerIdentity::from_test_seed([0x99; 32]);
    let (id, plan_hash, host, package_sha, manifest) = consent.scope();
    let claim = OwnerApprovalWireV1 {
        approver_id: owner.signer_id().into(),
        install_id: id,
        plan_digest: plan_hash,
        ssh_host_sha256: host,
        manifest_fingerprint: manifest,
        previous: OwnerFloorWire {
            generation: 0,
            sequence: 0,
            version: [0; 3],
            manifest_fingerprint: [0; 32],
            chain_tip: [0; 32],
        },
        proposed: OwnerFloorWire {
            generation: 1,
            sequence: 9,
            version: [0, 3, 13],
            manifest_fingerprint: manifest,
            chain_tip: [0x66; 32],
        },
        staged_package_sha256: package_sha,
        staged_package_size: artifact.len() as u64,
        expected_witness_revision: 0,
        approval_nonce: consent.nonce(),
        issued_at_unix: NOW,
        expires_at_unix: NOW + 60,
    };
    let signature = owner
        .sign_consented_claim(&consent, &claim, NOW + 2)
        .unwrap();
    assert_eq!(
        claim.verify_detached(&signature, &owner.public_key_ed25519(), NOW + 2),
        Ok(())
    );
    let mut wrong = claim.clone();
    wrong.ssh_host_sha256[0] ^= 1;
    assert_eq!(
        owner.sign_consented_claim(&consent, &wrong, NOW + 2),
        Err(OwnerIdentityError::InvalidScope),
    );
    assert_eq!(
        claim.verify_detached(&signature, &[0; 32], NOW + 2),
        Err(WireError::UntrustedOwner)
    );
    assert_eq!(
        owner.sign_consented_claim(&consent, &claim, NOW + 60),
        Err(OwnerIdentityError::InvalidClaim(WireError::Expired)),
    );
}

#[test]
fn legacy_loopback_only_plan_is_never_admissible_for_public_node() {
    use crate::server_bootstrap_lab::ProposedChange;
    let (plan, signed, trusted, artifact) = fixture();
    assert_eq!(plan.schema_version, 2);
    assert!(plan
        .proposed_changes
        .contains(&ProposedChange::BindPublicUdp43871WithoutChangingFirewall,));
    assert!(!plan
        .proposed_changes
        .contains(&ProposedChange::EnableLocalLoopbackOnly));

    let input = B0Input {
        install_id: plan.install_id,
        target: plan.target.clone(),
        package: plan.package.clone(),
    };
    let wire = b"SSH-server-public-key-fixture-never-real".to_vec();
    let legacy = preview(&input, &mut VerifiedKexFixture { wire }, || false).unwrap();
    assert!(legacy.verify_unchanged());
    assert_eq!(legacy.schema_version, 1);
    assert!(matches!(
        prepare(legacy, &signed, &trusted, &artifact),
        Err(ReviewError::InvalidPreview),
    ));

    let mut dropped = plan.clone();
    dropped
        .proposed_changes
        .retain(|change| *change != ProposedChange::BindPublicUdp43871WithoutChangingFirewall);
    dropped.plan_blake3 = dropped.computed_digest();
    assert!(!dropped.verify_unchanged());
    assert!(matches!(
        prepare(dropped, &signed, &trusted, &artifact),
        Err(ReviewError::InvalidPreview),
    ));
}
