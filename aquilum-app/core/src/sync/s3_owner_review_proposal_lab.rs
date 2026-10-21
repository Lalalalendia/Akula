//! E3h: untrusted, bounded OFFLINE owner-review proposal import.
//!
//! This is NOT an authenticated invitation, S3 content proof, account
//! attestation, source of trusted keys, or authority to enroll the Vault.
//! It gives the future Native review UI a canonical typed candidate to
//! compare with a SEPARATELY received owner fingerprint. The displayed
//! candidate must NEVER be presented as independently approved.

use serde::Deserialize;
use super::{IndependentlyApprovedJoin, S3Keyspace, VaultId};
use super::s3_owner_manual_fingerprint_lab::{
    IndependentOwnerFingerprintError, verify_independently_entered_owner_fingerprint,
};
use super::s3_publisher_policy_lab::MAX_OWNER_KEYS;
use std::collections::BTreeSet;

pub const MAX_OWNER_REVIEW_PROPOSAL_BYTES: usize = 4096;
const OWNER_KEY_BYTES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerReviewProposalError {
    Oversized,
    Malformed,
    WrongProvider,
    InvalidScope,
    InvalidOwners,
    InvalidPins,
    Comparison(IndependentOwnerFingerprintError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireProposal {
    schema: u16,
    provider: String,
    endpoint: String,
    region: String,
    force_path_style: bool,
    bucket: String,
    prefix: String,
    vault_id: VaultId,
    genesis_root_blake3: String,
    genesis_policy_blake3: String,
    owner_keys_hex: Vec<String>,
    owner_quorum: usize,
}

/// Parsed public candidate metadata, NOT independently authenticated.
pub struct OwnerReviewProposal {
    bucket: String,
    keyspace: S3Keyspace,
    genesis_root_blake3: String,
    genesis_policy_blake3: String,
    owner_keys: Vec<[u8; OWNER_KEY_BYTES]>,
    owner_quorum: usize,
}

fn lower_hex(value: &str, n: usize) -> bool {
    value.len() == n && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.bytes().any(|b| b != b'0')
}

fn decode_owner_key(hex: &str) -> Option<[u8; OWNER_KEY_BYTES]> {
    if hex.len() != OWNER_KEY_BYTES * 2 { return None; }
    let mut key = [0u8; OWNER_KEY_BYTES];
    for (dst, two) in key.iter_mut().zip(hex.as_bytes().chunks_exact(2)) {
        let hi = (two[0] as char).to_digit(16)?;
        let lo = (two[1] as char).to_digit(16)?;
        if !(two[0].is_ascii_digit() || (b'a'..=b'f').contains(&two[0]))
            || !(two[1].is_ascii_digit() || (b'a'..=b'f').contains(&two[1])) {
            return None;
        }
        *dst = ((hi << 4) | lo) as u8;
    }
    key.iter().any(|b| *b != 0).then_some(key)
}

impl OwnerReviewProposal {
    /// Parse bytes from an untrusted file or transport; NO filesystem or
    /// S3 access here. A valid proposal still requires owner-side proof.
    pub fn parse_untrusted_json(bytes: &[u8]) -> Result<Self, OwnerReviewProposalError> {
        if bytes.len() > MAX_OWNER_REVIEW_PROPOSAL_BYTES {
            return Err(OwnerReviewProposalError::Oversized);
        }
        let p: WireProposal = serde_json::from_slice(bytes)
            .map_err(|_| OwnerReviewProposalError::Malformed)?;
        if p.schema != 1
            || p.provider != "yandex-object-storage"
            || p.endpoint != "https://storage.yandexcloud.net"
            || p.region != "ru-central1"
            || !p.force_path_style {
            return Err(OwnerReviewProposalError::WrongProvider);
        }
        if p.bucket.len() < 3 || p.bucket.len() > 63
            || !p.bucket.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            || !p.bucket.as_bytes().last().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            || !p.bucket.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err(OwnerReviewProposalError::InvalidScope);
        }
        let keyspace = S3Keyspace::new(&p.prefix, p.vault_id)
            .map_err(|_| OwnerReviewProposalError::InvalidScope)?;
        if !lower_hex(&p.genesis_root_blake3, 64)
            || !lower_hex(&p.genesis_policy_blake3, 64) {
            return Err(OwnerReviewProposalError::InvalidPins);
        }
        if p.owner_keys_hex.is_empty() || p.owner_keys_hex.len() > MAX_OWNER_KEYS
            || p.owner_quorum == 0 || p.owner_quorum > p.owner_keys_hex.len() {
            return Err(OwnerReviewProposalError::InvalidOwners);
        }
        let owner_keys = p.owner_keys_hex.iter()
            .map(|v| decode_owner_key(v).ok_or(OwnerReviewProposalError::InvalidOwners))
            .collect::<Result<Vec<_>, _>>()?;
        if owner_keys.iter().copied().collect::<BTreeSet<_>>().len() != owner_keys.len() {
            return Err(OwnerReviewProposalError::InvalidOwners);
        }
        Ok(Self {
            bucket: p.bucket, keyspace,
            genesis_root_blake3: p.genesis_root_blake3,
            genesis_policy_blake3: p.genesis_policy_blake3,
            owner_keys, owner_quorum: p.owner_quorum,
        })
    }

    pub fn bucket(&self) -> &str { &self.bucket }
    pub fn keyspace(&self) -> &S3Keyspace { &self.keyspace }

    /// Returns a borrowed view for an E3h comparison; the receiver challenge
    /// MUST have been issued by this device's E3e session for this scope.
    pub fn approval_for_challenge<'a>(&'a self, challenge_hex: &'a str)
        -> IndependentlyApprovedJoin<'a> {
        IndependentlyApprovedJoin {
            bucket: &self.bucket,
            keyspace: &self.keyspace,
            pinned_genesis_root_blake3: &self.genesis_root_blake3,
            pinned_genesis_policy_blake3: &self.genesis_policy_blake3,
            trusted_owner_keys: &self.owner_keys,
            owner_quorum: self.owner_quorum,
            recipient_challenge_hex: challenge_hex,
        }
    }

    /// Compare ONLY to the code entered from a different authenticated
    /// owner-controlled channel. Do not display the locally computed value
    /// next to the entry field or autofill it from this proposal.
    pub fn verify_independent_owner_code(
        &self, challenge_hex: &str, typed_from_owner: &str,
    ) -> Result<(), OwnerReviewProposalError> {
        verify_independently_entered_owner_fingerprint(
            &self.approval_for_challenge(challenge_hex), typed_from_owner,
        ).map_err(OwnerReviewProposalError::Comparison)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::s3_owner_manual_fingerprint_lab::owner_join_fingerprint_hex;

    fn fixture() -> serde_json::Value {
        serde_json::json!({
            "schema":1,
            "provider":"yandex-object-storage",
            "endpoint":"https://storage.yandexcloud.net",
            "region":"ru-central1",
            "force_path_style":true,
            "bucket":"akula-review-only",
            "prefix":"akula/owner-review",
            "vault_id":VaultId::new(),
            "genesis_root_blake3":"a".repeat(64),
            "genesis_policy_blake3":"b".repeat(64),
            "owner_keys_hex": ["1".repeat(64), "2".repeat(64)],
            "owner_quorum":2
        })
    }
    fn parse(value: serde_json::Value) -> Result<OwnerReviewProposal,OwnerReviewProposalError>{
        OwnerReviewProposal::parse_untrusted_json(&serde_json::to_vec(&value).unwrap())
    }

    #[test]
    fn proposal_import_does_not_replace_owner_out_of_band_approval() {
        let candidate = parse(fixture()).unwrap();
        let challenge = "a".repeat(64);
        let expected = owner_join_fingerprint_hex(
            &candidate.approval_for_challenge(&challenge),
        ).unwrap();
        assert_eq!(expected.len(),64);
        assert_eq!(candidate.verify_independent_owner_code(&challenge,&expected),Ok(()));
        let wrong = if expected == "f".repeat(64) {"e".repeat(64)} else {"f".repeat(64)};
        assert_eq!(candidate.verify_independent_owner_code(&challenge,&wrong),
            Err(OwnerReviewProposalError::Comparison(IndependentOwnerFingerprintError::Mismatch)));
        assert!(candidate.verify_independent_owner_code(&"c".repeat(64), &expected).is_err());
    }

    #[test]
    fn candidate_parser_rejects_untrusted_scope_authority_and_oversize() {
        let mut p=fixture();
        p["bucket"]=serde_json::json!("akula-other");
        assert!(parse(p.clone()).is_ok()); // untrusted is only a candidate
        p["endpoint"]=serde_json::json!("https://other.example");
        assert!(matches!(parse(p),Err(OwnerReviewProposalError::WrongProvider)));
        let mut p=fixture();
        p["owner_keys_hex"]=serde_json::json!(["1".repeat(64),"1".repeat(64)]);
        assert!(matches!(parse(p),Err(OwnerReviewProposalError::InvalidOwners)));
        let mut p=fixture();
        p["genesis_root_blake3"]=serde_json::json!("x".repeat(64));
        assert!(matches!(parse(p),Err(OwnerReviewProposalError::InvalidPins)));
        let mut p=fixture();
        p["unknown_authority"]=serde_json::json!(true);
        assert!(matches!(parse(p),Err(OwnerReviewProposalError::Malformed)));
        assert_eq!(
            OwnerReviewProposal::parse_untrusted_json(&vec![b' '; MAX_OWNER_REVIEW_PROPOSAL_BYTES+1])
                .err(), Some(OwnerReviewProposalError::Oversized),
        );
    }
}
