//! B1b4a: correlate signed owner approval, root-pinned B1a ELF and a
//! separately signed witness CAS claim. This is a READ-ONLY laboratory.
//! Trusted before/after witness state and a fresh challenge MUST originate
//! outside SSH, staged files and IPC, not from caller-supplied data.
//! No local hash chain, public key or receipt alone proves external monotonicity.
//! There is NO install, activation, privileged IPC, rollback, or production grant.
use crate::server_installer_pinned_release_lab::{PinError, RootVerifiedFd};
use crate::server_owner_approval_lab::{
    verify_owner_approval, ApprovalError, ExternalApprovalScope,
    SignedOwnerApproval, TrustedOwnerApprover,
};
use crate::server_release_floor_lab::ProposedFloor;
use crate::server_staging_lab::StageReceipt;
use crate::server_witness_lab::WitnessCheckpoint;
use ring::{digest, signature};
const DOMAIN: &[u8] = b"AKULA-B1B4A-INERT-WITNESS-CAS-V1\0";
const OWNER_DOMAIN: &[u8] = b"AKULA-B1B4A-SIGNED-OWNER-V1\0";

#[derive(Clone, Debug)]
pub struct WitnessClaim {
    pub witness_id: String,
    pub before: WitnessCheckpoint,
    pub after: WitnessCheckpoint,
    pub install_id: [u8;16],
    pub plan_digest: [u8;32],
    pub host_sha256: [u8;32],
    pub manifest_fingerprint: [u8;32],
    pub package_sha256: [u8;32],
    pub signed_owner_sha256: [u8;32],
    pub approval_nonce: [u8;16],
    pub challenge: [u8;32],
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
}
fn push_head(out:&mut Vec<u8>, c:WitnessCheckpoint) {
    let h=c.accepted_floor;
    out.extend_from_slice(&c.revision.to_be_bytes());
    out.extend_from_slice(&c.record_tip);
    out.extend_from_slice(&h.generation.to_be_bytes());
    out.extend_from_slice(&h.chain_tip);
    out.extend_from_slice(&h.floor.sequence.to_be_bytes());
    for v in [h.floor.version.major,h.floor.version.minor,h.floor.version.patch] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&h.floor.signed_manifest_fingerprint);
}
impl WitnessClaim {
    pub fn canonical_bytes(&self)->Vec<u8> {
        let mut b=Vec::with_capacity(512);
        b.extend_from_slice(DOMAIN);
        b.push(1);
        b.push(u8::try_from(self.witness_id.len()).unwrap_or(u8::MAX));
        b.extend_from_slice(self.witness_id.as_bytes());
        push_head(&mut b,self.before);
        push_head(&mut b,self.after);
        b.extend_from_slice(&self.install_id);
        b.extend_from_slice(&self.plan_digest);
        b.extend_from_slice(&self.host_sha256);
        b.extend_from_slice(&self.manifest_fingerprint);
        b.extend_from_slice(&self.package_sha256);
        b.extend_from_slice(&self.signed_owner_sha256);
        b.extend_from_slice(&self.approval_nonce);
        b.extend_from_slice(&self.challenge);
        b.extend_from_slice(&self.issued_at_unix.to_be_bytes());
        b.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        b
    }
}
#[derive(Clone,Debug)]
pub struct SignedWitnessClaim {
    pub claim:WitnessClaim,
    pub signature:[u8;64],
}
#[derive(Clone,Debug)]
pub struct TrustedWitnessSigner {
    pub witness_id:String,
    pub ed25519_public:[u8;32],
}
#[derive(Debug)]
pub enum FenceError {
    Pinned(PinError),Owner(ApprovalError),PackageMismatch,
    UntrustedWitness,BadWitnessSignature,ExpiredWitness,
    WitnessScope,StaleIndependentHead,
}
/// Still an inert diagnostic. This is never an authorization capability.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct InertFenceEvidence {
    pub matched_revision:u64,
    pub package_sha256:[u8;32],
    pub requires_fresh_atomic_commit_check:bool,
}
pub fn signed_owner_sha256(owner:&SignedOwnerApproval)->[u8;32] {
    let mut h=digest::Context::new(&digest::SHA256);
    h.update(OWNER_DOMAIN);
    h.update(&owner.claim.canonical_bytes());
    h.update(&owner.signature);
    h.finish().as_ref().try_into().expect("SHA-256 output length")
}
/// Source of checkpoints MUST be a protected independent CAS authority.
/// Accepting a prospective installer's values would destroy this boundary.
/// A successful match NEVER authorizes installation; recheck at actual commit.
#[allow(clippy::too_many_arguments)]
pub fn check_inert_cas_fence(
    pinned:&RootVerifiedFd,
    owner:&SignedOwnerApproval,
    owner_signer:&TrustedOwnerApprover,
    scope:ExternalApprovalScope,
    stage:&StageReceipt,
    proposed:ProposedFloor,
    witness:&SignedWitnessClaim,
    witness_signer:&TrustedWitnessSigner,
    trusted_before:WitnessCheckpoint,
    observed_after:WitnessCheckpoint,
    root_challenge:[u8;32],
    now_unix:u64,
)->Result<InertFenceEvidence,FenceError>{
    pinned.valid_at(now_unix).map_err(FenceError::Pinned)?;
    if stage.verified_package!=pinned.receipt {
        return Err(FenceError::PackageMismatch);
    }
    verify_owner_approval(owner,owner_signer,scope,stage,proposed,now_unix)
        .map_err(FenceError::Owner)?;
    let c=&witness.claim;
    let id=c.witness_id.as_bytes();
    if witness_signer.ed25519_public==[0;32]
        || c.witness_id!=witness_signer.witness_id
        || id.is_empty() || id.len()>64
        || !id.iter().all(|x| x.is_ascii_alphanumeric()
            || matches!(*x,b'.'|b'-'|b'_'))
    {return Err(FenceError::UntrustedWitness);}
    signature::UnparsedPublicKey::new(
        &signature::ED25519,witness_signer.ed25519_public)
        .verify(&c.canonical_bytes(),&witness.signature)
        .map_err(|_|FenceError::BadWitnessSignature)?;
    let span=c.expires_at_unix.checked_sub(c.issued_at_unix)
        .ok_or(FenceError::ExpiredWitness)?;
    if span==0 || span>60 || now_unix<c.issued_at_unix
        || now_unix>=c.expires_at_unix
    {return Err(FenceError::ExpiredWitness);}
    if root_challenge==[0;32] || c.challenge!=root_challenge
        || c.install_id!=scope.install_id.0
        || c.plan_digest!=scope.approved_plan_digest.0
        || c.host_sha256!=scope.pinned_host_key_sha256
        || c.manifest_fingerprint!=pinned.receipt.signed_manifest_fingerprint
        || c.package_sha256!=pinned.receipt.package_sha256
        || c.signed_owner_sha256!=signed_owner_sha256(owner)
        || c.approval_nonce!=owner.claim.approval_nonce
    {return Err(FenceError::WitnessScope);}
    if trusted_before.revision!=scope.expected_witness_revision
        || trusted_before.accepted_floor!=scope.independently_pinned_previous_head
        || c.before!=trusted_before || c.after!=observed_after
        || c.after.revision!=c.before.revision.checked_add(1)
            .ok_or(FenceError::StaleIndependentHead)?
        || c.after.accepted_floor!=proposed.proposed_head
        || c.after.record_tip==[0;32] || c.after.record_tip==c.before.record_tip
        || (c.before.revision==0 && c.before!=WitnessCheckpoint::genesis())
        || (c.before.revision>0 && c.before.record_tip==[0;32])
    {return Err(FenceError::StaleIndependentHead);}
    pinned.valid_at(now_unix).map_err(FenceError::Pinned)?;
    Ok(InertFenceEvidence{
        matched_revision:c.after.revision,
        package_sha256:pinned.receipt.package_sha256,
        requires_fresh_atomic_commit_check:true,
    })
}
