//! Reuse the B1b4a fixture for OS-pinned CAS tests; no install actions.
//! No user host and no installation is involved.
#![cfg(all(target_os="linux",feature="server-bootstrap-b1b4b-os-witness-lab"))]
use aquilum_core::server_installer_cas_fence_lab::{
    check_inert_cas_fence,signed_owner_sha256,FenceError,
    InertFenceEvidence,SignedWitnessClaim,TrustedWitnessSigner,WitnessClaim,
};
use aquilum_core::server_installer_pinned_release_lab::{
    verify_root_staged,PinError,RootPinnedReleaseTrust,RootVerifiedFd,
};
use aquilum_core::server_owner_approval_lab::{
    ApprovalError,ExternalApprovalScope,SignedApprovalClaim,
    SignedOwnerApproval,TrustedOwnerApprover,
};
use aquilum_core::server_release_floor_lab::{
    LabPlanBinding,ProposedFloor,TrustedFloorHead,
};
use aquilum_core::server_release_lab::{
    verify_signed_release,ReleaseError,ReleaseFloor,ReleaseManifestV1,
    SignedRelease,TargetPlatform,TrustedReleaseSigner,Version,
};
use aquilum_core::server_staging_lab::{ApprovedPlanDigest,InstallId,StageReceipt};
use aquilum_core::server_witness_lab::WitnessCheckpoint;
use ring::{digest,signature::{self,KeyPair}};
use std::fs::{self,Permissions};
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;
const NOW:u64=1_800_000_000;
const CHALLENGE:[u8;32]=[0x90;32];
const NODE:&[u8]=b"synthetic signed root pinned Akula ELF-like bytes never executed";
fn key(v:u8)->signature::Ed25519KeyPair {
    signature::Ed25519KeyPair::from_seed_unchecked(&[v;32]).unwrap()
}
fn write(p:&std::path::Path,b:&[u8]){
    fs::write(p,b).unwrap();
    fs::set_permissions(p,Permissions::from_mode(0o400)).unwrap();
}
struct Case {
    _dir:TempDir,pinned:RootVerifiedFd,
    owner:SignedOwnerApproval,owner_signer:TrustedOwnerApprover,
    scope:ExternalApprovalScope,stage:StageReceipt,proposed:ProposedFloor,
    witness:SignedWitnessClaim,witness_signer:TrustedWitnessSigner,
    before:WitnessCheckpoint,after:WitnessCheckpoint,challenge:[u8;32],
}
impl Case {
    fn new()->Self{
        assert_eq!(rustix::process::geteuid().as_raw(),0);
        let dir=tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(),Permissions::from_mode(0o700)).unwrap();
        let rk=key(0x31);
        let hash:[u8;32]=digest::digest(&digest::SHA256,NODE)
            .as_ref().try_into().unwrap();
        let m=ReleaseManifestV1{
            platform:TargetPlatform::LinuxX86_64,
            version:Version{major:1,minor:0,patch:0},
            release_sequence:17,package_size:NODE.len() as u64,
            not_before_unix:NOW-60,expires_unix:NOW+120,
            package_sha256:hash,signer_id:"b1b4a-test-release".into(),
        };
        let sig:[u8;64]=rk.sign(&m.canonical_signed_bytes())
            .as_ref().try_into().unwrap();
        let signer=TrustedReleaseSigner{
            signer_id:m.signer_id.clone(),
            ed25519_public:rk.public_key().as_ref().try_into().unwrap(),
        };
        let release=verify_signed_release(
            &SignedRelease{manifest:m.clone(),signature:sig},
            &signer,ReleaseFloor::initial(),NOW).unwrap();
        write(&dir.path().join("node"),NODE);
        write(&dir.path().join("manifest"),&m.canonical_signed_bytes());
        write(&dir.path().join("signature"),&sig);
        let trust=RootPinnedReleaseTrust{
            trusted_signer:signer,previous_floor:ReleaseFloor::initial(),
            approved_manifest_fingerprint:
                release.release_floor_candidate().signed_manifest_fingerprint,
            approved_package_sha256:hash,approved_release_sequence:17,
        };
        let pinned=verify_root_staged(dir.path(),&trust,NOW).unwrap();
        let install_id=InstallId([0x11;16]);
        let plan=ApprovedPlanDigest([0x22;32]);
        let stage=StageReceipt{
            install_id,approved_plan_digest:plan,
            verified_package:pinned.receipt.clone(),idempotent_replay:false,
        };
        let binding=LabPlanBinding{
            install_id,plan_digest:plan,pinned_host_key_sha256:[0x33;32],
            signed_manifest_fingerprint:pinned.receipt.signed_manifest_fingerprint,
        };
        let before=WitnessCheckpoint::genesis();
        let newhead=TrustedFloorHead{
            generation:1,floor:pinned.receipt.proposed_floor,chain_tip:[0x44;32],
        };
        let proposed=ProposedFloor{
            proposed_head:newhead,idempotent_replay:false,
            requires_external_witness:true,
        };
        let scope=ExternalApprovalScope{
            install_id,approved_plan_digest:plan,
            pinned_host_key_sha256:binding.pinned_host_key_sha256,
            independently_pinned_previous_head:before.accepted_floor,
            expected_witness_revision:0,
        };
        let ok=key(0x32);
        let owner_signer=TrustedOwnerApprover{
            approver_id:"b1b4a-test-owner".into(),
            public_key_ed25519:ok.public_key().as_ref().try_into().unwrap(),
        };
        let claim=SignedApprovalClaim{
            approver_id:owner_signer.approver_id.clone(),
            scope_binding:binding,previous_head:before.accepted_floor,
            proposed_head:newhead,staged_package_sha256:hash,
            staged_package_size:NODE.len() as u64,expected_witness_revision:0,
            approval_nonce:[0x55;16],
            issued_at_unix:NOW-10,expires_at_unix:NOW+45,
        };
        let owner=SignedOwnerApproval{
            signature:ok.sign(&claim.canonical_bytes()).as_ref().try_into().unwrap(),
            claim,
        };
        let after=WitnessCheckpoint{
            revision:1,accepted_floor:newhead,record_tip:[0x66;32],
        };
        let wk=key(0x34);
        let witness_signer=TrustedWitnessSigner{
            witness_id:"independent-ci-witness".into(),
            ed25519_public:wk.public_key().as_ref().try_into().unwrap(),
        };
        let wc=WitnessClaim{
            witness_id:witness_signer.witness_id.clone(),
            before,after,install_id:install_id.0,plan_digest:plan.0,
            host_sha256:binding.pinned_host_key_sha256,
            manifest_fingerprint:pinned.receipt.signed_manifest_fingerprint,
            package_sha256:hash,signed_owner_sha256:signed_owner_sha256(&owner),
            approval_nonce:owner.claim.approval_nonce,challenge:CHALLENGE,
            issued_at_unix:NOW-5,expires_at_unix:NOW+20,
        };
        let witness=SignedWitnessClaim{
            signature:wk.sign(&wc.canonical_bytes()).as_ref().try_into().unwrap(),
            claim:wc,
        };
        Self{_dir:dir,pinned,owner,owner_signer,scope,stage,proposed,
            witness,witness_signer,before,after,challenge:CHALLENGE}
    }
    fn check(&self,now:u64)->Result<InertFenceEvidence,FenceError>{
        check_inert_cas_fence(
            &self.pinned,&self.owner,&self.owner_signer,self.scope,
            &self.stage,self.proposed,&self.witness,&self.witness_signer,
            self.before,self.after,self.challenge,now)
    }
    fn resign(&mut self){
        self.witness.signature=key(0x34)
            .sign(&self.witness.claim.canonical_bytes()).as_ref().try_into().unwrap();
    }
}

use aquilum_core::server_installer_os_witness_lab::{OsWitnessError, OsWitnessLab};
use aquilum_core::server_witness_lab::WitnessError;
use std::path::Path;

fn provision() -> (TempDir, TempDir) {
    let trust = tempfile::tempdir().unwrap();
    let journal = tempfile::tempdir().unwrap();
    for dir in [&trust, &journal] {
        fs::set_permissions(dir.path(), Permissions::from_mode(0o700)).unwrap();
    }
    for (name, bytes) in [
        ("witness.seed", vec![0x34; 32]),
        ("witness.pub", key(0x34).public_key().as_ref().to_vec()),
        ("enrollment", b"AKULA-B1B4B-CI-ENROLLED\n".to_vec()),
    ] {
        let path = trust.path().join(name);
        fs::write(&path, bytes).unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o600)).unwrap();
    }
    (trust, journal)
}
fn open_actor(trust: &Path, journal: &Path) -> OsWitnessLab {
    OsWitnessLab::open(trust, journal).unwrap()
}
#[test]
fn signed_witness_reflects_an_actual_durable_new_cas() {
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    let signed = actor.attest_new_record(
        &c.pinned, &c.owner, &c.owner_signer, c.scope,
        &c.stage, c.proposed, c.before, c.challenge, NOW,
    ).unwrap();
    let current = actor.current_checkpoint().unwrap();
    assert_eq!(current, signed.claim.after);
    let evidence = check_inert_cas_fence(
        &c.pinned, &c.owner, &c.owner_signer, c.scope,
        &c.stage, c.proposed, &signed, &actor.trusted_witness(),
        c.before, current, c.challenge, NOW,
    ).unwrap();
    assert!(evidence.requires_fresh_atomic_commit_check);
    assert_eq!(evidence.matched_revision, 1);
    assert_eq!(fs::read_dir(journal.path()).unwrap().count(), 1);
}
#[test]
fn lost_ack_never_issues_a_second_signed_attestation() {
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    actor.attest_new_record(&c.pinned, &c.owner, &c.owner_signer,
        c.scope, &c.stage, c.proposed, c.before, c.challenge, NOW).unwrap();
    assert!(matches!(actor.attest_new_record(&c.pinned, &c.owner, &c.owner_signer,
        c.scope, &c.stage, c.proposed, c.before, c.challenge, NOW),
        Err(OsWitnessError::StaleCheckpoint)));
    assert_eq!(fs::read_dir(journal.path()).unwrap().count(), 1);
}
#[test]
fn restart_loads_the_exact_persisted_checkpoint() {
    let c = Case::new();
    let (trust, journal) = provision();
    let accepted;
    {
        let actor = open_actor(trust.path(), journal.path());
        accepted = actor.attest_new_record(&c.pinned, &c.owner, &c.owner_signer,
            c.scope, &c.stage, c.proposed, c.before, c.challenge, NOW)
            .unwrap().claim.after;
    }
    let reopened = open_actor(trust.path(), journal.path());
    assert_eq!(reopened.current_checkpoint().unwrap(), accepted);
    assert!(matches!(reopened.attest_new_record(&c.pinned, &c.owner,
        &c.owner_signer, c.scope, &c.stage, c.proposed,
        c.before, c.challenge, NOW), Err(OsWitnessError::StaleCheckpoint)));
}
#[test]
fn fake_owner_signature_rejected_before_any_record() {
    let mut c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    c.owner.signature[0] ^= 1;
    assert!(matches!(actor.attest_new_record(&c.pinned, &c.owner,
        &c.owner_signer, c.scope, &c.stage, c.proposed,
        c.before, c.challenge, NOW),
        Err(OsWitnessError::Witness(WitnessError::Approval(
            ApprovalError::BadSignature)))));
    assert_eq!(fs::read_dir(journal.path()).unwrap().count(), 0);
}
#[test]
fn forged_verified_receipt_does_not_mutate_cas() {
    let mut c = Case::new();
    let (trust, journal) = provision();
    c.stage.verified_package.package_sha256[0] ^= 1;
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(actor.attest_new_record(&c.pinned, &c.owner,
        &c.owner_signer, c.scope, &c.stage, c.proposed,
        c.before, c.challenge, NOW), Err(OsWitnessError::PackageMismatch)));
    assert_eq!(fs::read_dir(journal.path()).unwrap().count(), 0);
}
#[test]
fn empty_challenge_is_not_recorded() {
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(actor.attest_new_record(&c.pinned, &c.owner,
        &c.owner_signer, c.scope, &c.stage, c.proposed,
        c.before, [0; 32], NOW), Err(OsWitnessError::InvalidChallenge)));
    assert_eq!(fs::read_dir(journal.path()).unwrap().count(), 0);
}
#[test]
fn signer_key_mismatch_refused_before_opening_journal() {
    let (trust, journal) = provision();
    let public = trust.path().join("witness.pub");
    fs::write(&public, [0xBB; 32]).unwrap();
    assert!(matches!(OsWitnessLab::open(trust.path(), journal.path()),
        Err(OsWitnessError::SignatureKey)));
}
#[test]
fn signer_symlink_and_bad_mode_are_rejected() {
    let (trust, journal) = provision();
    let seed = trust.path().join("witness.seed");
    fs::set_permissions(&seed, Permissions::from_mode(0o644)).unwrap();
    assert!(OsWitnessLab::open(trust.path(), journal.path()).is_err());
    fs::set_permissions(&seed, Permissions::from_mode(0o600)).unwrap();
    let public = trust.path().join("witness.pub");
    fs::remove_file(&public).unwrap();
    std::os::unix::fs::symlink("witness.seed", &public).unwrap();
    assert!(OsWitnessLab::open(trust.path(), journal.path()).is_err());
}
#[test]
fn missing_provisioning_marker_cannot_automatically_enroll_genesis() {
    let (trust, journal) = provision();
    fs::remove_file(trust.path().join("enrollment")).unwrap();
    assert!(OsWitnessLab::open(trust.path(), journal.path()).is_err());
}
#[test]
fn tampering_the_durable_witness_record_refuses_readback() {
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    actor.attest_new_record(&c.pinned, &c.owner, &c.owner_signer,
        c.scope, &c.stage, c.proposed, c.before, c.challenge, NOW).unwrap();
    drop(actor);
    let file = journal.path().join("witness-0000000000000001.record");
    let mut bytes = fs::read(&file).unwrap();
    bytes[35] ^= 1;
    fs::write(&file, bytes).unwrap();
    let restarted = open_actor(trust.path(), journal.path());
    assert!(matches!(restarted.current_checkpoint(),
        Err(OsWitnessError::Witness(WitnessError::CorruptRecord))));
}
#[test]
fn expired_pinned_artifact_never_reaches_the_witness_ledger() {
    let c = Case::new();
    let (trust, journal) = provision();
    let actor = open_actor(trust.path(), journal.path());
    assert!(matches!(actor.attest_new_record(&c.pinned, &c.owner,
        &c.owner_signer, c.scope, &c.stage, c.proposed,
        c.before, c.challenge, NOW + 120),
        Err(OsWitnessError::Pinned(PinError::Release(ReleaseError::Expired)))));
    assert_eq!(fs::read_dir(journal.path()).unwrap().count(), 0);
}

#[cfg(feature = "server-bootstrap-b1b4b1-witness-ipc-lab")]
#[test]
fn ci_optional_real_witness_ipc_checkpoint_fixture() {
    // Absolutely no mutation unless the disposable GitHub runner explicitly
    // selects this exact, test-only UID0 bootstrap operation.
    if std::env::var("AKULA_B1B4B1_CI_SEED").ok().as_deref()
        != Some("APPROVED_DISPOSABLE_ROOT_CAS_ONLY")
    {
        return;
    }
    assert_eq!(std::env::var("GITHUB_ACTIONS").ok().as_deref(), Some("true"));
    assert_eq!(rustix::process::geteuid().as_raw(), 0);
    let actor = OsWitnessLab::open(
        std::path::Path::new("/run/akula-b1b4b1-lab/trust"),
        std::path::Path::new("/run/akula-b1b4b1-lab/journal"),
    ).unwrap();
    assert_eq!(actor.current_checkpoint().unwrap(), WitnessCheckpoint::genesis());
    let c = Case::new();
    let attested = actor.attest_new_record(
        &c.pinned, &c.owner, &c.owner_signer, c.scope,
        &c.stage, c.proposed, c.before, c.challenge, NOW,
    ).unwrap();
    assert_eq!(attested.claim.after.revision, 1);
    assert_eq!(attested.claim.after.accepted_floor.floor.sequence, 17);
    assert_eq!(actor.current_checkpoint().unwrap(), attested.claim.after);
}
