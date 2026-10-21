//! Actual UID0 pinned FD and synthetic independent owner/CAS signers.
//! No user host and no installation is involved.
#![cfg(all(target_os="linux",feature="server-bootstrap-b1b4a-cas-fence-lab"))]
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
#[test]
fn all_three_valid_signers_and_actual_inode_still_grant_no_install(){
    let c=Case::new();let proof=c.check(NOW).unwrap();
    assert_eq!(proof.matched_revision,1);
    assert!(proof.requires_fresh_atomic_commit_check);
    assert_eq!(proof.package_sha256,c.pinned.receipt.package_sha256);
}
#[test]
fn tampered_cas_signature_refused(){
    let mut c=Case::new();c.witness.signature[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::BadWitnessSignature)));
}
#[test]
fn wrong_cas_key_refused(){
    let mut c=Case::new();c.witness_signer.ed25519_public[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::BadWitnessSignature)));
}
#[test]
fn forged_owner_signature_refused(){
    let mut c=Case::new();c.owner.signature[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::Owner(ApprovalError::BadSignature))));
}
#[test]
fn wrong_actual_package_receipt_refused(){
    let mut c=Case::new();c.stage.verified_package.package_sha256[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::PackageMismatch)));
}
#[test]
fn independently_signed_wrong_host_refused(){
    let mut c=Case::new();c.witness.claim.host_sha256[0]^=1;c.resign();
    assert!(matches!(c.check(NOW),Err(FenceError::WitnessScope)));
}
#[test]
fn independently_signed_wrong_owner_digest_refused(){
    let mut c=Case::new();c.witness.claim.signed_owner_sha256[0]^=1;c.resign();
    assert!(matches!(c.check(NOW),Err(FenceError::WitnessScope)));
}
#[test]
fn challenge_must_be_fresh_and_nonzero(){
    let mut c=Case::new();c.challenge[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::WitnessScope)));
    c.challenge=[0;32];
    assert!(matches!(c.check(NOW),Err(FenceError::WitnessScope)));
}
#[test]
fn signed_witness_expiry_is_independent_from_owner_expiry(){
    let c=Case::new();
    assert!(matches!(c.check(NOW+20),Err(FenceError::ExpiredWitness)));
}
#[test]
fn independently_observed_stale_head_is_refused(){
    let mut c=Case::new();c.after.record_tip[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::StaleIndependentHead)));
}
#[test]
fn rolled_back_before_checkpoint_is_refused(){
    let mut c=Case::new();c.before.revision=1;
    assert!(matches!(c.check(NOW),Err(FenceError::StaleIndependentHead)));
}
#[test]
fn signed_epoch_jump_is_refused(){
    let mut c=Case::new();c.witness.claim.after.revision=2;
    c.after=c.witness.claim.after;c.resign();
    assert!(matches!(c.check(NOW),Err(FenceError::StaleIndependentHead)));
}
#[test]
fn expired_pinned_release_refused_even_with_owner_and_cas_synthetics(){
    let c=Case::new();
    assert!(matches!(c.check(NOW+120),
        Err(FenceError::Pinned(PinError::Release(ReleaseError::Expired)))));
}
#[test]
fn owner_host_scope_mismatch_refused(){
    let mut c=Case::new();c.scope.pinned_host_key_sha256[0]^=1;
    assert!(matches!(c.check(NOW),Err(FenceError::Owner(ApprovalError::ScopeMismatch))));
}
