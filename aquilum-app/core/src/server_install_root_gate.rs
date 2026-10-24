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
    let span = evidence.expires_at_unix.checked_sub(evidence.issued_at_unix);
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
    let verifier = signature::UnparsedPublicKey::new(
        &signature::ED25519,
        pins.attestor_ed25519,
    );
    if verifier
        .verify(&evidence.canonical_signed_bytes(), &evidence.signature)
        .is_err()
    {
        return RootHelperReadiness::NeedsAdminAuthorization;
    }
    RootHelperReadiness::AuthenticatedPresenceOnly
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use uuid::Uuid;

    const NOW: u64 = 1_800_000_000;
    const HOLD: RootHelperReadiness = RootHelperReadiness::NeedsAdminAuthorization;

    fn signed_fixture() -> (ReviewEcho, RootHelperPins, RootHelperEvidence) {
        let key = Ed25519KeyPair::from_seed_unchecked(&[0x91; 32]).unwrap();
        let review = ReviewEcho {
            install_id: Uuid::from_bytes([0x11; 16]),
            plan_hash: [0x22; 32],
            ssh_host_key_sha256: [0x33; 32],
            package_sha256: [0x44; 32],
            review_nonce: [0x55; 16],
        };
        let pins = RootHelperPins {
            attestor_ed25519: key.public_key().as_ref().try_into().unwrap(),
            helper_sha256: [0x66; 32],
            owner_ed25519: [0x77; 32],
            root_policy_sha256: [0x88; 32],
        };
        let mut proof = RootHelperEvidence {
            install_id: *review.install_id.as_bytes(),
            plan_blake3: review.plan_hash,
            ssh_host_sha256: review.ssh_host_key_sha256,
            package_sha256: review.package_sha256,
            review_nonce: review.review_nonce,
            helper_sha256: pins.helper_sha256,
            owner_ed25519: pins.owner_ed25519,
            root_policy_sha256: pins.root_policy_sha256,
            issued_at_unix: NOW - 5,
            expires_at_unix: NOW + 45,
            signature: [0; 64],
        };
        proof.signature = key.sign(&proof.canonical_signed_bytes()).as_ref()
            .try_into().unwrap();
        (review, pins, proof)
    }

    #[test]
    fn only_pre_enrolled_signed_presence_is_observed() {
        let (review, pins, proof) = signed_fixture();
        assert_eq!(assess_pre_enrolled_helper(&review, &pins, None, NOW), HOLD);
        assert_eq!(
            assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW),
            RootHelperReadiness::AuthenticatedPresenceOnly,
        );
    }

    #[test]
    fn mismatched_scope_and_unpinned_keys_hold() {
        for variant in 0..11 {
            let (mut review, mut pins, mut proof) = signed_fixture();
            match variant {
                0 => proof.install_id[0] ^= 1,
                1 => proof.plan_blake3[0] ^= 1,
                2 => proof.ssh_host_sha256[0] ^= 1,
                3 => proof.package_sha256[0] ^= 1,
                4 => proof.review_nonce[0] ^= 1,
                5 => proof.helper_sha256[0] ^= 1,
                6 => proof.owner_ed25519[0] ^= 1,
                7 => proof.root_policy_sha256[0] ^= 1,
                8 => proof.signature[0] ^= 1,
                9 => pins.attestor_ed25519 = [0; 32],
                10 => review.review_nonce[0] ^= 1,
                _ => unreachable!(),
            }
            if variant <= 7 {
                // A root signer can sign a different *valid* candidate;
                // that must still be refused against independently pinned scope.
                let signer = Ed25519KeyPair::from_seed_unchecked(&[0x91; 32]).unwrap();
                proof.signature = signer.sign(&proof.canonical_signed_bytes())
                    .as_ref().try_into().unwrap();
            }
            assert_eq!(
                assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW),
                HOLD,
                "variant {variant}",
            );
        }
    }

    #[test]
    fn expired_future_or_unbounded_evidence_holds() {
        let (review, pins, proof) = signed_fixture();
        assert_eq!(
            assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW + 45),
            HOLD,
        );
        assert_eq!(
            assess_pre_enrolled_helper(&review, &pins, Some(&proof), NOW - 6),
            HOLD,
        );
        let mut too_long = proof;
        too_long.expires_at_unix = NOW + MAX_TTL_SECONDS;
        assert_eq!(
            assess_pre_enrolled_helper(&review, &pins, Some(&too_long), NOW),
            HOLD,
        );
    }
}
