//! B1-E2E-07: the ONE portable B1b1b owner-approval signing format.
//!
//! Windows signs canonical claim bytes only after explicit immutable plan
//! review. Linux B1b1b verifies the SAME canonical bytes and independently
//! checks staged release, scope, root-owned witness and its one-use CAS.
//! Neither valid signature nor this module can install or start Node.
//! Owner signing keys MUST be provisioned outside untrusted SSH or package.
use ring::signature;

const DOMAIN: &[u8] = b"Akula B1b1b owner approval v1: inert witness CAS\0";
pub const MAX_VALIDITY_SECONDS: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OwnerFloorWire {
    pub generation: u64,
    pub sequence: u64,
    pub version: [u32; 3],
    pub manifest_fingerprint: [u8; 32],
    pub chain_tip: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerApprovalWireV1 {
    pub approver_id: String,
    pub install_id: [u8; 16],
    pub plan_digest: [u8; 32],
    pub ssh_host_sha256: [u8; 32],
    pub manifest_fingerprint: [u8; 32],
    pub previous: OwnerFloorWire,
    pub proposed: OwnerFloorWire,
    pub staged_package_sha256: [u8; 32],
    pub staged_package_size: u64,
    pub expected_witness_revision: u64,
    pub approval_nonce: [u8; 16],
    pub issued_at_unix: u64,
    pub expires_at_unix: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    MalformedClaim,
    NotYetValid,
    Expired,
    UntrustedOwner,
    BadSignature,
}

impl OwnerApprovalWireV1 {
    /// This byte string MUST match the existing Linux B1b1b signing domain
    /// and field order. No locale, JSON maps, arbitrary paths or shell input.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(512);
        out.extend_from_slice(DOMAIN);
        out.push(1);
        out.push(u8::try_from(self.approver_id.len()).unwrap_or(u8::MAX));
        out.extend_from_slice(self.approver_id.as_bytes());
        out.extend_from_slice(&self.install_id);
        out.extend_from_slice(&self.plan_digest);
        out.extend_from_slice(&self.ssh_host_sha256);
        out.extend_from_slice(&self.manifest_fingerprint);
        for head in [self.previous, self.proposed] {
            out.extend_from_slice(&head.generation.to_be_bytes());
            out.extend_from_slice(&head.sequence.to_be_bytes());
            for version in head.version {
                out.extend_from_slice(&version.to_be_bytes());
            }
            out.extend_from_slice(&head.manifest_fingerprint);
            out.extend_from_slice(&head.chain_tip);
        }
        out.extend_from_slice(&self.staged_package_sha256);
        out.extend_from_slice(&self.staged_package_size.to_be_bytes());
        out.extend_from_slice(&self.expected_witness_revision.to_be_bytes());
        out.extend_from_slice(&self.approval_nonce);
        out.extend_from_slice(&self.issued_at_unix.to_be_bytes());
        out.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        out
    }

    pub fn validate_time_and_shape(&self, now_unix: u64) -> Result<(), WireError> {
        let id = self.approver_id.as_bytes();
        if id.is_empty()
            || id.len() > 64
            || !id
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || matches!(*c, b'.' | b'-' | b'_'))
            || self.install_id == [0; 16]
            || self.plan_digest == [0; 32]
            || self.ssh_host_sha256 == [0; 32]
            || self.manifest_fingerprint == [0; 32]
            || self.staged_package_sha256 == [0; 32]
            || self.staged_package_size == 0
            || self.approval_nonce == [0; 16]
        {
            return Err(WireError::MalformedClaim);
        }
        let duration = self
            .expires_at_unix
            .checked_sub(self.issued_at_unix)
            .ok_or(WireError::MalformedClaim)?;
        if duration == 0 || duration > MAX_VALIDITY_SECONDS {
            return Err(WireError::MalformedClaim);
        }
        if now_unix < self.issued_at_unix {
            return Err(WireError::NotYetValid);
        }
        if now_unix >= self.expires_at_unix {
            return Err(WireError::Expired);
        }
        Ok(())
    }

    /// Portable owner signature check, never a privileged install decision.
    /// Root MUST also compare floor, stage, host, owner policy and atomic CAS.
    pub fn verify_detached(
        &self,
        signature_bytes: &[u8; 64],
        independently_pinned_key: &[u8; 32],
        now_unix: u64,
    ) -> Result<(), WireError> {
        self.validate_time_and_shape(now_unix)?;
        if independently_pinned_key == &[0; 32] {
            return Err(WireError::UntrustedOwner);
        }
        signature::UnparsedPublicKey::new(&signature::ED25519, independently_pinned_key)
            .verify(&self.canonical_bytes(), signature_bytes)
            .map_err(|_| WireError::BadSignature)
    }
}

pub mod decode;

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const NOW: u64 = 1_800_000_000;

    fn candidate() -> OwnerApprovalWireV1 {
        let previous = OwnerFloorWire {
            generation: 0,
            sequence: 0,
            version: [0; 3],
            manifest_fingerprint: [0; 32],
            chain_tip: [0; 32],
        };
        OwnerApprovalWireV1 {
            approver_id: "akula-owner-test".into(),
            install_id: [0x11; 16],
            plan_digest: [0x22; 32],
            ssh_host_sha256: [0x33; 32],
            manifest_fingerprint: [0x44; 32],
            previous,
            proposed: OwnerFloorWire {
                generation: 1,
                sequence: 7,
                version: [0, 3, 13],
                manifest_fingerprint: [0x44; 32],
                chain_tip: [0x55; 32],
            },
            staged_package_sha256: [0x66; 32],
            staged_package_size: 32768,
            expected_witness_revision: 0,
            approval_nonce: [0x77; 16],
            issued_at_unix: NOW - 1,
            expires_at_unix: NOW + 60,
        }
    }

    #[test]
    fn detached_owner_signature_is_bounded_and_scope_sensitive() {
        let key = Ed25519KeyPair::from_seed_unchecked(&[0x99; 32]).unwrap();
        let claim = candidate();
        let signed = key.sign(&claim.canonical_bytes());
        let signature: [u8; 64] = signed.as_ref().try_into().unwrap();
        let public: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
        assert_eq!(claim.verify_detached(&signature, &public, NOW), Ok(()));
        for variant in 0..9 {
            let mut changed = claim.clone();
            match variant {
                0 => changed.install_id[0] ^= 1,
                1 => changed.plan_digest[0] ^= 1,
                2 => changed.ssh_host_sha256[0] ^= 1,
                3 => changed.manifest_fingerprint[0] ^= 1,
                4 => changed.previous.chain_tip[0] ^= 1,
                5 => changed.proposed.chain_tip[0] ^= 1,
                6 => changed.staged_package_sha256[0] ^= 1,
                7 => changed.approval_nonce[0] ^= 1,
                8 => changed.expected_witness_revision = 1,
                _ => unreachable!(),
            }
            assert_eq!(
                changed.verify_detached(&signature, &public, NOW),
                Err(WireError::BadSignature),
                "field variant {variant}",
            );
        }
        assert_eq!(
            claim.verify_detached(&signature, &[0; 32], NOW),
            Err(WireError::UntrustedOwner)
        );
        assert_eq!(
            claim.verify_detached(&signature, &public, NOW + 60),
            Err(WireError::Expired)
        );
    }

    #[test]
    fn invalid_owner_signing_claim_fails_before_signature_check() {
        let mut bad = candidate();
        bad.approver_id = "../root".into();
        assert_eq!(
            bad.validate_time_and_shape(NOW),
            Err(WireError::MalformedClaim)
        );
        let mut bad = candidate();
        bad.approval_nonce = [0; 16];
        assert_eq!(
            bad.validate_time_and_shape(NOW),
            Err(WireError::MalformedClaim)
        );
        let mut bad = candidate();
        bad.expires_at_unix = NOW + MAX_VALIDITY_SECONDS + 100;
        assert_eq!(
            bad.validate_time_and_shape(NOW),
            Err(WireError::MalformedClaim)
        );
        let mut bad = candidate();
        bad.issued_at_unix = NOW + 2;
        assert_eq!(
            bad.validate_time_and_shape(NOW),
            Err(WireError::NotYetValid)
        );
    }
}
