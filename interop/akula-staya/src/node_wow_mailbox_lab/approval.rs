//! Detached operator consent for the first WOW mailbox policy publication.
//! This is an INERT CI LAB verifier, modelled on B1d0's separate
//! owner-approval and CAS-witness authorities. Signature verification is
//! necessary, but it is never sufficient to publish a file or activate Node.
use super::policy::MailRoleClaims;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"Akula/NodeV1/WOW/MailFirstPublishApproval/lab-v1\0";
const MAX_APPROVAL_BYTES: usize = 512;
const MAX_APPROVAL_WINDOW_MS: i64 = 15 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MailFirstApprovalClaims {
    pub version: u8,
    pub operation: u8,
    pub node_public: [u8; 32],
    pub node_endpoint: [u8; 32],
    pub signed_policy_digest: [u8; 32],
    pub policy_epoch: u64,
    pub expected_witness_revision: u64,
    pub approval_nonce: [u8; 16],
    pub helper_challenge: [u8; 32],
    pub issued_at_ms: i64,
    pub valid_until_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct SignedFirstApproval {
    claims: MailFirstApprovalClaims,
    signature: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailApprovalHold {
    InvalidFormat,
    InvalidScope,
    ExpiredOrNotYetValid,
    UntrustedOwnerSignature,
    StaleWitnessRevision,
}

/// This *proposal* is not a witness receipt or a file write capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InertMailCasProposal {
    pub approved_digest: [u8; 32],
    pub approval_nonce: [u8; 16],
    pub challenge: [u8; 32],
    pub expected_witness_revision: u64,
    pub required_next_revision: u64,
    pub requires_independent_cas_commit: bool,
}

pub fn sign_first_mail_approval_for_lab(
    owner: &SigningKey,
    claims: MailFirstApprovalClaims,
) -> Vec<u8> {
    let body = postcard::to_stdvec(&claims).expect("fixed-size owner approval");
    let mut signed = Vec::with_capacity(DOMAIN.len() + body.len());
    signed.extend_from_slice(DOMAIN);
    signed.extend_from_slice(&body);
    postcard::to_stdvec(&SignedFirstApproval {
        claims,
        signature: owner.sign(&signed).to_bytes().to_vec(),
    }).expect("fixed-size signature")
}

/// Caller MUST have obtained the owner key and mail policy claims from the
/// CURRENT independently pinned Node RootTrust. An expected witness revision
/// from a caller/network is NOT authoritative: the result explicitly requires
/// a separate trusted CAS actor to compare-and-commit that revision.
pub(super) fn verify_first_mail_approval(
    raw: &[u8],
    policy_bytes: &[u8],
    policy: MailRoleClaims,
    operator_public: [u8; 32],
    expected_witness_revision: u64,
    now_ms: i64,
) -> Result<InertMailCasProposal, MailApprovalHold> {
    if raw.len() > MAX_APPROVAL_BYTES || raw.is_empty() || operator_public == [0; 32] {
        return Err(MailApprovalHold::InvalidFormat);
    }
    let signed: SignedFirstApproval =
        postcard::from_bytes(raw).map_err(|_| MailApprovalHold::InvalidFormat)?;
    if postcard::to_stdvec(&signed).map_err(|_| MailApprovalHold::InvalidFormat)? != raw
        || signed.signature.len() != 64
    {
        return Err(MailApprovalHold::InvalidFormat);
    }
    let claim = signed.claims;
    if claim.version != 1
        || claim.operation != 1 // first publication, never reinstall/update
        || claim.node_public != policy.node_public
        || claim.node_endpoint != policy.node_endpoint
        || claim.policy_epoch != policy.policy_epoch
        || claim.policy_epoch != 1
        || claim.signed_policy_digest != *blake3::hash(policy_bytes).as_bytes()
        || claim.approval_nonce == [0; 16]
        || claim.helper_challenge == [0; 32]
        || claim.issued_at_ms < 0
        || claim.valid_until_ms <= claim.issued_at_ms
    {
        return Err(MailApprovalHold::InvalidScope);
    }
    let validity = claim.valid_until_ms
        .checked_sub(claim.issued_at_ms)
        .ok_or(MailApprovalHold::InvalidScope)?;
    if validity > MAX_APPROVAL_WINDOW_MS {
        return Err(MailApprovalHold::InvalidScope);
    }
    if now_ms < claim.issued_at_ms || now_ms >= claim.valid_until_ms {
        return Err(MailApprovalHold::ExpiredOrNotYetValid);
    }
    if claim.expected_witness_revision != expected_witness_revision {
        return Err(MailApprovalHold::StaleWitnessRevision);
    }
    let verifying = VerifyingKey::from_bytes(&operator_public)
        .map_err(|_| MailApprovalHold::UntrustedOwnerSignature)?;
    let body = postcard::to_stdvec(&claim).map_err(|_| MailApprovalHold::InvalidFormat)?;
    let mut message = Vec::with_capacity(DOMAIN.len() + body.len());
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(&body);
    let signature = Signature::from_slice(&signed.signature)
        .map_err(|_| MailApprovalHold::UntrustedOwnerSignature)?;
    verifying.verify(&message, &signature)
        .map_err(|_| MailApprovalHold::UntrustedOwnerSignature)?;
    Ok(InertMailCasProposal {
        approved_digest: claim.signed_policy_digest,
        approval_nonce: claim.approval_nonce,
        challenge: claim.helper_challenge,
        expected_witness_revision,
        required_next_revision: expected_witness_revision
            .checked_add(1).ok_or(MailApprovalHold::StaleWitnessRevision)?,
        requires_independent_cas_commit: true,
    })
}
