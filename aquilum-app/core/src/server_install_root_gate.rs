//! B1-E2E-07: non-authorizing, pre-enrolled root-helper presence evidence.
//!
//! A successful SSH KEX, sudo -n -l, or user-supplied executable path is
//! NEVER privilege. Only independently pinned root-attestor evidence can
//! establish that a previously enrolled helper answered this exact review.
//! This gate cannot bootstrap an empty VPS, issue a sudo command or install.
use crate::server_release_lab::review::ReviewEcho;
use ring::signature;

const DOMAIN: &[u8] = b"Akula B1-E2E-07 preenrolled root helper evidence v1\0";
const MAX_TTL_SECONDS: u64 = 90;

/// All pins come from protected client-side enrollment, not SSH host facts,
/// remote helper output, a package candidate, or the caller's sudo listing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootHelperPins {
    pub attestor_ed25519: [u8; 32],
    pub helper_sha256: [u8; 32],
    pub owner_ed25519: [u8; 32],
    pub root_policy_sha256: [u8; 32],
}

/// Fixed-size, domain-separated helper observation. A future root helper must
/// independently assert its OS ownership and policy before signing this wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RootHelperEvidence {
    pub install_id: [u8; 16],
    pub plan_blake3: [u8; 32],
    pub ssh_host_sha256: [u8; 32],
    pub package_sha256: [u8; 32],
    pub review_nonce: [u8; 16],
    pub helper_sha256: [u8; 32],
    pub owner_ed25519: [u8; 32],
    pub root_policy_sha256: [u8; 32],
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
    pub signature: [u8; 64],
}

impl RootHelperEvidence {
    /// No dynamic paths, shell commands, sudo permission strings or URLs.
    pub fn canonical_signed_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(DOMAIN.len() + 16 * 2 + 32 * 6 + 16);
        bytes.extend_from_slice(DOMAIN);
        bytes.extend_from_slice(&self.install_id);
        bytes.extend_from_slice(&self.plan_blake3);
        bytes.extend_from_slice(&self.ssh_host_sha256);
        bytes.extend_from_slice(&self.package_sha256);
        bytes.extend_from_slice(&self.review_nonce);
        bytes.extend_from_slice(&self.helper_sha256);
        bytes.extend_from_slice(&self.owner_ed25519);
        bytes.extend_from_slice(&self.root_policy_sha256);
        bytes.extend_from_slice(&self.issued_at_unix.to_be_bytes());
        bytes.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        bytes
    }
}

/// Even authenticated presence is NOT authorization for B1d0/root CAS,
/// process launch, service activation, or the native Install Node button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootHelperReadiness {
    NeedsAdminAuthorization,
    AuthenticatedPresenceOnly,
}

/// Only admits evidence from a separately root-enrolled helper. There is no
/// permissive fallback based on the user's general sudo rights.
pub fn assess_pre_enrolled_helper(
    reviewed: &ReviewEcho,
    pins: &RootHelperPins,
    evidence: Option<&RootHelperEvidence>,
    now_unix: u64,
) -> RootHelperReadiness {
    let Some(evidence) = evidence else {
        return RootHelperReadiness::NeedsAdminAuthorization;
    };
    let nonzero = |bytes: &[u8]| bytes.iter().any(|b| *b != 0);
    let span = evidence
        .expires_at_unix
        .checked_sub(evidence.issued_at_unix);
    if !nonzero(&pins.attestor_ed25519)
        || !nonzero(&pins.helper_sha256)
        || !nonzero(&pins.owner_ed25519)
        || !nonzero(&pins.root_policy_sha256)
        || !nonzero(&reviewed.review_nonce)
        || !nonzero(&evidence.signature)
        || evidence.install_id != *reviewed.install_id.as_bytes()
        || evidence.plan_blake3 != reviewed.plan_hash
        || evidence.ssh_host_sha256 != reviewed.ssh_host_key_sha256
        || evidence.package_sha256 != reviewed.package_sha256
        || evidence.review_nonce != reviewed.review_nonce
        || evidence.helper_sha256 != pins.helper_sha256
        || evidence.owner_ed25519 != pins.owner_ed25519
        || evidence.root_policy_sha256 != pins.root_policy_sha256
        || !matches!(span, Some(1..=MAX_TTL_SECONDS))
        || now_unix < evidence.issued_at_unix
        || now_unix >= evidence.expires_at_unix
    {
        return RootHelperReadiness::NeedsAdminAuthorization;
    }
    let verifier = signature::UnparsedPublicKey::new(&signature::ED25519, pins.attestor_ed25519);
    if verifier
        .verify(&evidence.canonical_signed_bytes(), &evidence.signature)
        .is_err()
    {
        return RootHelperReadiness::NeedsAdminAuthorization;
    }
    RootHelperReadiness::AuthenticatedPresenceOnly
}

#[cfg(test)]
mod tests;
