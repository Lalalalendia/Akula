//! E3h: independently compared owner/Vault join fingerprint (research only).
//!
//! The fingerprint contains ONLY public authority metadata. It is not a
//! signature, an authentication secret, a proof that an independent channel
//! was used, an S3 provider identity, or a production writer capability.
//!
//! The RECEIVER must type/scan the expected full fingerprint from an
//! authenticated owner-controlled channel, never copy/prefill it from the
//! candidate Yandex objects or the proposed join invitation itself.

use std::collections::BTreeSet;

use super::IndependentlyApprovedJoin;
use super::s3_publisher_policy_lab::MAX_OWNER_KEYS;

const DOMAIN: &str = "akula.yc-s3.independent-owner-join-approval.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndependentOwnerFingerprintError {
    InvalidApproval,
    InvalidIndependentEntry,
    Mismatch,
}

fn lower_hex(value: &str, len: usize, require_nonzero: bool) -> bool {
    value.len() == len
        && value.bytes().all(|byte| byte.is_ascii_digit()
            || (b'a'..=b'f').contains(&byte))
        && (!require_nonzero || value.bytes().any(|byte| byte != b'0'))
}

fn append_len_prefixed(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Public owner-side transcript fingerprint. Both participants must first
/// agree through a *separately authenticated* channel on this full value.
///
/// Binds the exact Yandex provider context, bucket, VaultId/keyspace, gen-1
/// root BLAKE3, publisher policy BLAKE3, distinct owner keys + threshold
/// and THIS RECEIVER'S fresh challenge. Sorting keys avoids a cosmetic
/// ordering-dependent mismatch without changing the approved keyset.
pub fn owner_join_fingerprint_hex(
    approved: &IndependentlyApprovedJoin<'_>,
) -> Result<String, IndependentOwnerFingerprintError> {
    if approved.bucket.len() < 3
        || approved.bucket.len() > 63
        || !approved.bucket.bytes().all(|c| c.is_ascii_lowercase()
            || c.is_ascii_digit() || c == b'-')
        || !lower_hex(approved.pinned_genesis_root_blake3, 64, true)
        || !lower_hex(approved.pinned_genesis_policy_blake3, 64, true)
        || !lower_hex(approved.recipient_challenge_hex, 64, true)
        || !(1..=MAX_OWNER_KEYS).contains(&approved.trusted_owner_keys.len())
        || !(1..=approved.trusted_owner_keys.len()).contains(&approved.owner_quorum)
        || approved.trusted_owner_keys.iter().any(|key| key.iter().all(|b| *b == 0))
        || approved.trusted_owner_keys.iter().copied()
            .collect::<BTreeSet<_>>().len() != approved.trusted_owner_keys.len()
    {
        return Err(IndependentOwnerFingerprintError::InvalidApproval);
    }
    let mut owners = approved.trusted_owner_keys.to_vec();
    owners.sort_unstable();
    let mut h = blake3::Hasher::new_derive_key(DOMAIN);
    let vault = approved.keyspace.vault_id().to_string();
    let key = approved.keyspace.current_head_key();
    for item in [
        b"yandex-object-storage".as_slice(),
        b"https://storage.yandexcloud.net".as_slice(),
        b"ru-central1".as_slice(),
        b"force-path-style:true".as_slice(),
        approved.bucket.as_bytes(),
        vault.as_bytes(),
        key.as_bytes(),
        approved.pinned_genesis_root_blake3.as_bytes(),
        approved.pinned_genesis_policy_blake3.as_bytes(),
        approved.recipient_challenge_hex.as_bytes(),
    ] {
        append_len_prefixed(&mut h, item);
    }
    append_len_prefixed(&mut h, &(approved.owner_quorum as u64).to_le_bytes());
    append_len_prefixed(&mut h, &(owners.len() as u64).to_le_bytes());
    for owner in owners {
        append_len_prefixed(&mut h, &owner);
    }
    Ok(h.finalize().to_hex().to_string())
}

/// A strict FULL 256-bit comparison. This must be fed by a new manual
/// owner-controlled input; the receiver must NEVER auto-fill from the same
/// untrusted candidate it is trying to validate.
pub fn verify_independently_entered_owner_fingerprint(
    approved: &IndependentlyApprovedJoin<'_>,
    entered_out_of_band_hex: &str,
) -> Result<(), IndependentOwnerFingerprintError> {
    if !lower_hex(entered_out_of_band_hex, 64, true) {
        return Err(IndependentOwnerFingerprintError::InvalidIndependentEntry);
    }
    if owner_join_fingerprint_hex(approved)? != entered_out_of_band_hex {
        return Err(IndependentOwnerFingerprintError::Mismatch);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{S3Keyspace, VaultId};

    #[test]
    fn full_owner_digest_binds_vault_bucket_pins_quorum_and_challenge() {
        let ns = S3Keyspace::new("akula/e3h-fingerprint", VaultId::new()).unwrap();
        let owners = [[9u8; 32], [7u8; 32]];
        let reversed = [[7u8; 32], [9u8; 32]];
        let challenge = "a".repeat(64);
        let root = "b".repeat(64);
        let policy = "c".repeat(64);
        let approval = IndependentlyApprovedJoin {
            bucket: "akula-e3h-review",
            keyspace: &ns,
            pinned_genesis_root_blake3: &root,
            pinned_genesis_policy_blake3: &policy,
            trusted_owner_keys: &owners,
            owner_quorum: 2,
            recipient_challenge_hex: &challenge,
        };
        let fp = owner_join_fingerprint_hex(&approval).unwrap();
        assert_eq!(fp.len(), 64);
        assert!(lower_hex(&fp, 64, true));
        assert!(verify_independently_entered_owner_fingerprint(&approval, &fp).is_ok());
        assert_eq!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            trusted_owner_keys: &reversed, ..approval
        }).unwrap(), fp);
        assert_ne!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            owner_quorum: 1, ..approval
        }).unwrap(), fp);
        assert_ne!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            bucket: "akula-other-review", ..approval
        }).unwrap(), fp);
        assert_ne!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            pinned_genesis_root_blake3: &"d".repeat(64), ..approval
        }).unwrap(), fp);
        assert_ne!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            pinned_genesis_policy_blake3: &"e".repeat(64), ..approval
        }).unwrap(), fp);
        assert_ne!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            recipient_challenge_hex: &"f".repeat(64), ..approval
        }).unwrap(), fp);
        let other_ns = S3Keyspace::new("akula/e3h-other-vault", VaultId::new()).unwrap();
        assert_ne!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            keyspace: &other_ns, ..approval
        }).unwrap(), fp);
        assert!(matches!(
            verify_independently_entered_owner_fingerprint(&approval, &"0".repeat(64)),
            Err(IndependentOwnerFingerprintError::Mismatch),
        ));
        assert!(matches!(
            verify_independently_entered_owner_fingerprint(&approval, &fp.to_uppercase()),
            Err(IndependentOwnerFingerprintError::InvalidIndependentEntry),
        ));
        assert!(matches!(
            verify_independently_entered_owner_fingerprint(&approval, &fp[..32]),
            Err(IndependentOwnerFingerprintError::InvalidIndependentEntry),
        ));
    }

    #[test]
    fn duplicate_or_zero_owner_keys_never_form_a_valid_manual_review() {
        let ns = S3Keyspace::new("akula/e3h-review-invalid", VaultId::new()).unwrap();
        let invalid = [[1u8; 32], [1u8; 32]];
        let root = "b".repeat(64);
        let policy = "c".repeat(64);
        let challenge = "a".repeat(64);
        let approval = IndependentlyApprovedJoin {
            bucket: "akula-e3h-review", keyspace: &ns,
            pinned_genesis_root_blake3: &root,
            pinned_genesis_policy_blake3: &policy,
            trusted_owner_keys: &invalid,
            owner_quorum: 2,
            recipient_challenge_hex: &challenge,
        };
        assert!(matches!(owner_join_fingerprint_hex(&approval),
            Err(IndependentOwnerFingerprintError::InvalidApproval)));
        let zeros = [[0u8; 32]];
        assert!(matches!(owner_join_fingerprint_hex(&IndependentlyApprovedJoin {
            trusted_owner_keys: &zeros, owner_quorum: 1, ..approval
        }), Err(IndependentOwnerFingerprintError::InvalidApproval)));
    }
}
