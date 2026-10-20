//! E2a: offline publisher-policy rotation; does not change S3 or trusted state.
//! The PREVIOUS policy must be independently approved before calling this lab.
//! Successful signatures are observations, never a writer/Sync capability.

use std::collections::BTreeSet;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use super::{DecodedDsseEnvelope, DecodedSignature, S3Keyspace, VaultId};
use super::s3_signed_root_paev1::{pae, YANDEX_ENDPOINT, YANDEX_REGION};

pub const POLICY_PAYLOAD_TYPE: &str = "application/vnd.akula.publisher-policy+json;version=1";
pub const MAX_POLICY_BYTES: usize = 4096;
pub const MAX_POLICY_SIGNATURES: usize = 8;
pub const MAX_OWNER_KEYS: usize = 4;
pub const MAX_PUBLISHER_KEYS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyLabError {
    InvalidTrustedPolicy,
    InvalidEnvelope,
    InvalidPayloadType,
    OldOwnerQuorumMissing,
    InvalidCandidate,
    InvalidTransition,
    NewOwnerQuorumMissing,
}

/// A parsed policy is not trusted without an independent signed checkpoint.
/// E2a uses stable Rust struct-field JSON serialization for its lab digest;
/// production wire canonicalization and persistent receipts are separate gates.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublisherPolicyV1 {
    schema: u16,
    epoch: u64,
    provider: String,
    endpoint: String,
    region: String,
    force_path_style: bool,
    bucket: String,
    vault_id: VaultId,
    current_head_key: String,
    owner_keys: Vec<[u8; 32]>,
    owner_quorum: usize,
    publisher_keys: Vec<[u8; 32]>,
    publisher_quorum: usize,
    previous_policy_blake3: Option<String>,
}

impl PublisherPolicyV1 {
    pub fn parse_for_review(bytes: &[u8]) -> Result<Self, PolicyLabError> {
        if bytes.is_empty() || bytes.len() > MAX_POLICY_BYTES {
            return Err(PolicyLabError::InvalidCandidate);
        }
        serde_json::from_slice(bytes).map_err(|_| PolicyLabError::InvalidCandidate)
    }
    pub fn canonical_digest(&self) -> Result<String, PolicyLabError> {
        let b = serde_json::to_vec(self).map_err(|_| PolicyLabError::InvalidCandidate)?;
        if b.len() > MAX_POLICY_BYTES {
            return Err(PolicyLabError::InvalidCandidate);
        }
        Ok(blake3::hash(&b).to_hex().to_string())
    }
    pub fn epoch(&self) -> u64 { self.epoch }

    // E2b must source read-only publisher authority from the CURRENT
    // durable policy, never a cached policy from the remote S3 server.
    #[cfg(feature = "signed-vault-policy-receipt-lab")]
    pub(crate) fn trusted_publishers(&self) -> (&[[u8; 32]], usize) {
        (&self.publisher_keys, self.publisher_quorum)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyTransitionObservation {
    pub epoch: u64,
    pub policy_blake3: String,
    pub old_owner_quorum_satisfied: usize,
    pub new_owner_quorum_satisfied: usize,
}

fn valid_bucket(bucket: &str) -> bool {
    let b = bucket.as_bytes();
    (3..=63).contains(&b.len())
        && b.first().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b.last().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}
fn valid_keys(keys: &[[u8; 32]]) -> bool {
    keys.iter().all(|k| k.iter().any(|b| *b != 0))
        && keys.iter().copied().collect::<BTreeSet<_>>().len() == keys.len()
}
pub(super) fn valid_for_scope(p: &PublisherPolicyV1, bucket: &str, ns: &S3Keyspace) -> bool {
    let owners: BTreeSet<_> = p.owner_keys.iter().copied().collect();
    let n = p.publisher_keys.len();
    p.schema == 1 && p.epoch > 0
        && p.provider == "yandex-object-storage"
        && p.endpoint == YANDEX_ENDPOINT && p.region == YANDEX_REGION
        && p.force_path_style && valid_bucket(bucket)
        && p.bucket == bucket && p.vault_id == ns.vault_id()
        && p.current_head_key == ns.current_head_key()
        && (1..=MAX_OWNER_KEYS).contains(&p.owner_keys.len())
        && (1..=p.owner_keys.len()).contains(&p.owner_quorum)
        && valid_keys(&p.owner_keys)
        && n <= MAX_PUBLISHER_KEYS
        && (if n == 0 { p.publisher_quorum == 0 }
            else { (1..=n).contains(&p.publisher_quorum) })
        && valid_keys(&p.publisher_keys)
        && p.publisher_keys.iter().all(|k| !owners.contains(k))
        && (if p.epoch == 1 { p.previous_policy_blake3.is_none() }
            else { p.previous_policy_blake3.as_deref().is_some_and(|s|
                s.len() == 64 && s.bytes().all(|b|
                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) })
        && p.canonical_digest().is_ok()
}
fn verified_owner_count(
    data: &[u8], signatures: &[DecodedSignature<'_>], keys: &[[u8; 32]],
) -> usize {
    keys.iter().filter(|key| {
        let verifier = UnparsedPublicKey::new(&ED25519, *key);
        signatures.iter().any(|sig| verifier.verify(data, sig.sig).is_ok())
    }).count()
}

/// Offline TUF-style transition: exact same DSSE bytes must meet the OLD
/// owner threshold and the NEW owner threshold. Keyid is never authority.
///
/// PRECONDITION: old policy has independently verified origin and durable
/// status, which E2a does NOT prove. This function performs no mutation.
pub fn verify_policy_transition(
    old: &PublisherPolicyV1,
    bucket: &str,
    ns: &S3Keyspace,
    proposed: &DecodedDsseEnvelope<'_>,
) -> Result<PolicyTransitionObservation, PolicyLabError> {
    if !valid_for_scope(old, bucket, ns) {
        return Err(PolicyLabError::InvalidTrustedPolicy);
    }
    if proposed.payload_type != POLICY_PAYLOAD_TYPE {
        return Err(PolicyLabError::InvalidPayloadType);
    }
    if proposed.payload.is_empty() || proposed.payload.len() > MAX_POLICY_BYTES
        || proposed.signatures.is_empty()
        || proposed.signatures.len() > MAX_POLICY_SIGNATURES
        || proposed.signatures.iter().any(|s| s.sig.len() != 64
            || s.keyid.is_some_and(|hint| hint.len() > 128))
    {
        return Err(PolicyLabError::InvalidEnvelope);
    }
    let authenticated = pae(proposed.payload_type.as_bytes(), proposed.payload);
    let old_quorum = verified_owner_count(
        &authenticated, proposed.signatures, &old.owner_keys,
    );
    if old_quorum < old.owner_quorum {
        return Err(PolicyLabError::OldOwnerQuorumMissing);
    }
    // Only now interpret candidate-supplied owner keys; old quorum alone
    // can authorize its consideration, NOT skip new owner approval.
    let new = PublisherPolicyV1::parse_for_review(proposed.payload)?;
    if !valid_for_scope(&new, bucket, ns) {
        return Err(PolicyLabError::InvalidCandidate);
    }
    let old_hash = old.canonical_digest()
        .map_err(|_| PolicyLabError::InvalidTrustedPolicy)?;
    if old.epoch.checked_add(1) != Some(new.epoch)
        || new.previous_policy_blake3.as_deref() != Some(old_hash.as_str())
    {
        return Err(PolicyLabError::InvalidTransition);
    }
    let new_quorum = verified_owner_count(
        &authenticated, proposed.signatures, &new.owner_keys,
    );
    if new_quorum < new.owner_quorum {
        return Err(PolicyLabError::NewOwnerQuorumMissing);
    }
    Ok(PolicyTransitionObservation {
        epoch: new.epoch,
        policy_blake3: new.canonical_digest()?,
        old_owner_quorum_satisfied: old_quorum,
        new_owner_quorum_satisfied: new_quorum,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::{json, Value};

    fn signer(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
    }
    fn pk(s: &Ed25519KeyPair) -> [u8; 32] {
        s.public_key().as_ref().try_into().unwrap()
    }
    fn ns() -> S3Keyspace {
        S3Keyspace::new("akula/publisher-policy", VaultId::new()).unwrap()
    }
    fn genesis(ns: &S3Keyspace, owner: &Ed25519KeyPair) -> PublisherPolicyV1 {
        PublisherPolicyV1 {
            schema: 1, epoch: 1, provider: "yandex-object-storage".into(),
            endpoint: YANDEX_ENDPOINT.into(), region: YANDEX_REGION.into(),
            force_path_style: true, bucket: "akula-policy-test".into(),
            vault_id: ns.vault_id(), current_head_key: ns.current_head_key(),
            owner_keys: vec![pk(owner)], owner_quorum: 1,
            publisher_keys: vec![pk(&signer(99))], publisher_quorum: 1,
            previous_policy_blake3: None,
        }
    }
    fn rotate(old: &PublisherPolicyV1, keys: Vec<[u8; 32]>, q: usize)
        -> PublisherPolicyV1
    {
        let mut new = old.clone();
        new.epoch += 1;
        new.previous_policy_blake3 = Some(old.canonical_digest().unwrap());
        new.owner_keys = keys;
        new.owner_quorum = q;
        new
    }
    fn bytes(p: &PublisherPolicyV1) -> Vec<u8> {
        serde_json::to_vec(p).unwrap()
    }
    fn prove(
        old: &PublisherPolicyV1, ns: &S3Keyspace, payload: &[u8],
        signers: &[&Ed25519KeyPair],
    ) -> Result<PolicyTransitionObservation, PolicyLabError> {
        let data = pae(POLICY_PAYLOAD_TYPE.as_bytes(), payload);
        let raw: Vec<Vec<u8>> = signers.iter()
            .map(|k| k.sign(&data).as_ref().to_vec()).collect();
        let signs: Vec<_> = raw.iter().map(|s| DecodedSignature {
            sig: s, keyid: Some("arbitrary-untrusted-keyid"),
        }).collect();
        verify_policy_transition(old, "akula-policy-test", ns, &DecodedDsseEnvelope {
            payload_type: POLICY_PAYLOAD_TYPE, payload, signatures: &signs,
        })
    }

    #[test]
    fn complete_old_and_new_owner_quorums_approve_rotation() {
        let ns = ns();
        let (oldkey, alice, bob) = (signer(1), signer(2), signer(3));
        let old = genesis(&ns, &oldkey);
        let new = rotate(&old, vec![pk(&alice), pk(&bob)], 2);
        let result = prove(&old, &ns, &bytes(&new), &[&oldkey, &alice, &bob]).unwrap();
        assert_eq!(result.epoch, 2);
        assert_eq!(result.old_owner_quorum_satisfied, 1);
        assert_eq!(result.new_owner_quorum_satisfied, 2);
        assert_eq!(result.policy_blake3, new.canonical_digest().unwrap());
    }

    #[test]
    fn new_owner_alone_cannot_bootstrap_and_old_alone_cannot_rotate() {
        let ns = ns(); let (oldkey, next) = (signer(1), signer(2));
        let old = genesis(&ns, &oldkey);
        let new = rotate(&old, vec![pk(&next)], 1);
        assert_eq!(prove(&old, &ns, &bytes(&new), &[&next]),
            Err(PolicyLabError::OldOwnerQuorumMissing));
        assert_eq!(prove(&old, &ns, &bytes(&new), &[&oldkey]),
            Err(PolicyLabError::NewOwnerQuorumMissing));
    }

    #[test]
    fn delegated_publisher_cannot_approve_owner_rotation_or_revocation() {
        let ns = ns(); let (oldkey, next) = (signer(1), signer(2));
        let old = genesis(&ns, &oldkey);
        let mut new = rotate(&old, vec![pk(&next)], 1);
        new.publisher_keys.clear();
        new.publisher_quorum = 0;
        assert_eq!(prove(&old, &ns, &bytes(&new), &[&signer(99), &next]),
            Err(PolicyLabError::OldOwnerQuorumMissing));
        assert!(prove(&old, &ns, &bytes(&new), &[&oldkey, &next]).is_ok());
    }

    #[test]
    fn same_epoch_skip_and_wrong_predecessor_digest_fail_closed() {
        let ns = ns(); let key = signer(1);
        let old = genesis(&ns, &key);
        let mut p = rotate(&old, vec![pk(&key)], 1);
        p.epoch = 1;
        assert_eq!(prove(&old, &ns, &bytes(&p), &[&key]),
            Err(PolicyLabError::InvalidCandidate));
        p.epoch = 3;
        assert_eq!(prove(&old, &ns, &bytes(&p), &[&key]),
            Err(PolicyLabError::InvalidTransition));
        p.epoch = 2;
        p.previous_policy_blake3 = Some("0".repeat(64));
        assert_eq!(prove(&old, &ns, &bytes(&p), &[&key]),
            Err(PolicyLabError::InvalidTransition));
    }

    #[test]
    fn foreign_bucket_vault_namespace_and_provider_are_rejected() {
        let ns = ns(); let key = signer(1); let old = genesis(&ns, &key);
        for (field, value) in [
            ("bucket", json!("other-bucket")),
            ("vault_id", json!(VaultId::new())),
            ("current_head_key", json!("wrong/refs/current.json")),
            ("endpoint", json!("https://attacker.invalid")),
            ("region", json!("other-region")),
        ] {
            let p = rotate(&old, vec![pk(&key)], 1);
            let mut v: Value = serde_json::to_value(p).unwrap();
            v[field] = value;
            assert_eq!(prove(&old, &ns, &serde_json::to_vec(&v).unwrap(), &[&key]),
                Err(PolicyLabError::InvalidCandidate));
        }
    }

    #[test]
    fn duplicate_owner_keys_threshold_and_cross_role_keys_rejected() {
        let ns = ns(); let key = signer(1); let old = genesis(&ns, &key);
        let mut p = rotate(&old, vec![pk(&key), pk(&key)], 2);
        assert_eq!(prove(&old, &ns, &bytes(&p), &[&key]),
            Err(PolicyLabError::InvalidCandidate));
        p.owner_keys = vec![pk(&key)]; p.owner_quorum = 0;
        assert_eq!(prove(&old, &ns, &bytes(&p), &[&key]),
            Err(PolicyLabError::InvalidCandidate));
        p.owner_quorum = 1; p.publisher_keys = vec![pk(&key)];
        assert_eq!(prove(&old, &ns, &bytes(&p), &[&key]),
            Err(PolicyLabError::InvalidCandidate));
    }

    #[test]
    fn type_confusion_tampering_and_bounds_are_rejected() {
        let ns = ns(); let key = signer(1); let old = genesis(&ns, &key);
        let next = rotate(&old, vec![pk(&key)], 1); let body = bytes(&next);
        let data = pae(POLICY_PAYLOAD_TYPE.as_bytes(), &body);
        let sig = key.sign(&data).as_ref().to_vec();
        let signs = [DecodedSignature { sig: &sig, keyid: None }];
        assert_eq!(verify_policy_transition(&old, "akula-policy-test", &ns,
            &DecodedDsseEnvelope {
                payload_type: "application/json", payload: &body, signatures: &signs,
            }), Err(PolicyLabError::InvalidPayloadType));
        assert_eq!(verify_policy_transition(&old, "akula-policy-test", &ns,
            &DecodedDsseEnvelope {
                payload_type: POLICY_PAYLOAD_TYPE, payload: b"{}", signatures: &signs,
            }), Err(PolicyLabError::OldOwnerQuorumMissing));
        assert_eq!(prove(&old, &ns, &vec![b'x'; MAX_POLICY_BYTES+1], &[&key]),
            Err(PolicyLabError::InvalidEnvelope));
    }

    #[test]
    fn signed_unknown_or_duplicate_json_fields_fail_after_signature() {
        let ns = ns(); let key = signer(1); let old = genesis(&ns, &key);
        let p = rotate(&old, vec![pk(&key)], 1);
        let mut v = serde_json::to_value(&p).unwrap();
        v["superuser"] = json!(true);
        let bad = serde_json::to_vec(&v).unwrap();
        assert_eq!(prove(&old, &ns, &bad, &[&key]),
            Err(PolicyLabError::InvalidCandidate));
        let complete = String::from_utf8(bytes(&p)).unwrap();
        let duplicate = format!("{},\"epoch\":2}}", complete.trim_end_matches('}'));
        assert_eq!(prove(&old, &ns, duplicate.as_bytes(), &[&key]),
            Err(PolicyLabError::InvalidCandidate));
    }

    #[test]
    fn invalid_previous_trusted_policy_never_grants_transition() {
        let ns = ns(); let key = signer(1); let mut old = genesis(&ns, &key);
        old.owner_keys.clear();
        let new = rotate(&old, vec![pk(&key)], 1);
        assert_eq!(prove(&old, &ns, &bytes(&new), &[&key]),
            Err(PolicyLabError::InvalidTrustedPolicy));
    }

    #[test]
    fn two_signed_forks_are_not_adjudicated_by_stateless_verifier() {
        let ns = ns(); let key = signer(1); let old = genesis(&ns, &key);
        let mut a = rotate(&old, vec![pk(&key)], 1);
        let mut b = a.clone();
        a.publisher_keys = vec![pk(&signer(97))];
        b.publisher_keys = vec![pk(&signer(98))];
        assert!(prove(&old, &ns, &bytes(&a), &[&key]).is_ok());
        assert!(prove(&old, &ns, &bytes(&b), &[&key]).is_ok());
        assert_ne!(a.canonical_digest().unwrap(), b.canonical_digest().unwrap());
    }
}
