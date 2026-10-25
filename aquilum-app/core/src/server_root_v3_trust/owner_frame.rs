//! Read-only authenticated B1b1b owner-frame admission from fixed root trust.
//! This returns signed data for the EXISTING B0v3/B1d0 gates, never install
//! permission. The owner ID/key are loaded from root state, not from SSH.
use super::*;
use crate::server_owner_approval_lab::SignedOwnerApproval;
use crate::server_owner_approval_wire::decode::SignedOwnerWireFrame;
use crate::server_owner_approval_wire::WireError;
use std::time::{SystemTime, UNIX_EPOCH};

/// FD-scoped primitive: the trust descriptor must represent a protected,
/// independently enrolled UID0 directory. Only disposable fixtures in tests.
pub(super) fn verify_from_trust_fd(
    trust: &File,
    signed_frame: &[u8],
    now_unix: u64,
) -> Result<SignedOwnerApproval, RootEnrollmentError> {
    let enrolled = RootEnrolledV3::from_trust_fd(trust)?;
    let owner = enrolled.trusted_owner();
    let frame = SignedOwnerWireFrame::decode(signed_frame)
        .map_err(RootEnrollmentError::OwnerFrame)?;
    frame
        .verify_pinned_owner(&owner.approver_id, &owner.public_key_ed25519, now_unix)
        .map_err(RootEnrollmentError::OwnerFrame)?;
    let original = frame.canonical_frame();
    if original.as_slice() != signed_frame {
        return Err(RootEnrollmentError::OwnerFrame(WireError::MalformedClaim));
    }
    let legacy = frame.into_inert_linux_claim();
    // Encoding parity with the existing B1d0 signature verifier is mandatory.
    // No untrusted request or second serializer may change the signed scope.
    if legacy.claim.canonical_bytes() != &signed_frame[..signed_frame.len() - 64] {
        return Err(RootEnrollmentError::OwnerFrame(WireError::MalformedClaim));
    }
    Ok(legacy)
}

/// Fixed root-only production reader. A caller can provide only the signed
/// frame bytes; the enrolled identity and clock come from this host. The
/// existing B0v3 review, durable witness CAS and /opt writer remain separate.
pub(crate) fn load_verified_owner_frame(
    signed_frame: &[u8],
) -> Result<SignedOwnerApproval, RootEnrollmentError> {
    let trust = open_fixed_trust_fd()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RootEnrollmentError::InvalidOwnerClock)?
        .as_secs();
    verify_from_trust_fd(&trust, signed_frame, now)
}

#[cfg(test)]
mod tests;
