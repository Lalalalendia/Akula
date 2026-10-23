//! E1: offline-only DSSE PAE and Ed25519 verifier for Akula's V2 root.
//! Input is already-decoded DSSE data. Wire/base64 parsing and key custody
//! are intentionally OUT OF SCOPE until separately reviewed.
//! This experiment has no S3, file writes, key generation, or Sync authority.

use std::collections::BTreeSet;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::Deserialize;
use super::{S3Keyspace, S3PackedHeadV2, VaultId};

pub const ROOT_PAYLOAD_TYPE: &str =
    "application/vnd.akula.vault-root-attestation+json;version=1";
pub const YANDEX_ENDPOINT: &str = "https://storage.yandexcloud.net";
pub const YANDEX_REGION: &str = "ru-central1";
pub const MAX_PAYLOAD_BYTES: usize = 4096;
pub const MAX_SIGNATURES: usize = 4;
pub const MAX_TRUSTED_KEYS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignedRootError {
    InvalidApproval,
    InvalidEnvelope,
    InvalidType,
    InvalidSignature,
    MissingQuorum,
    InvalidSignedPayload,
    WrongScope,
    WrongRoot,
    MissingPredecessor,
    WrongAncestry,
}

/// The decoding layer must only supply exact bytes; keyid is advisory.
#[derive(Clone, Copy)]
pub struct DecodedSignature<'a> {
    pub sig: &'a [u8],
    pub keyid: Option<&'a str>,
}

pub struct DecodedDsseEnvelope<'a> {
    pub payload_type: &'a str,
    pub payload: &'a [u8],
    pub signatures: &'a [DecodedSignature<'a>],
}

/// The caller MUST obtain these keys and policy independently of S3.
pub struct ApprovedPublisherKeys<'a> {
    pub public_keys: &'a [[u8; 32]],
    pub quorum: usize,
}

pub struct ApprovedV2RootScope<'a> {
    pub bucket: &'a str,
    pub keyspace: &'a S3Keyspace,
    pub policy_epoch: u64,
    pub previous_trusted_root: Option<&'a S3PackedHeadV2>,
}

/// A read-only proof of signature validity for SUPPLIED in-memory root bytes,
/// not proof of pinned remote bytes, global freshness or writer permission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedRootObservation {
    pub root_blake3: String,
    pub generation: u64,
    pub verifying_distinct_keys: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AttestationV1 {
    schema: u16,
    provider: String,
    endpoint: String,
    region: String,
    force_path_style: bool,
    bucket: String,
    current_head_key: String,
    vault_id: VaultId,
    generation: u64,
    root_blake3: String,
    previous_root_blake3: Option<String>,
    policy_epoch: u64,
}

/// Standard DSSE v1 pre-authentication encoding, using BYTE lengths.
fn pae(kind: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(32 + kind.len() + payload.len());
    out.extend_from_slice(b"DSSEv1 ");
    out.extend_from_slice(kind.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(kind);
    out.push(b' ');
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.push(b' ');
    out.extend_from_slice(payload);
    out
}

fn valid_bucket(bucket: &str) -> bool {
    let b = bucket.as_bytes();
    (3..=63).contains(&b.len())
        && b.first().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b.last().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Offline-only verification. This does NOT pin remote S3 objects or update
/// existing S3V2TrustedAnchorJournal; the host must separately establish both.
pub fn verify_signed_root(
    env: &DecodedDsseEnvelope<'_>,
    root: &S3PackedHeadV2,
    approved: &ApprovedV2RootScope<'_>,
    policy: &ApprovedPublisherKeys<'_>,
) -> Result<SignedRootObservation, SignedRootError> {
    let keys = policy.public_keys;
    if keys.is_empty() || keys.len() > MAX_TRUSTED_KEYS
        || policy.quorum == 0 || policy.quorum > keys.len()
        || keys.iter().copied().collect::<BTreeSet<_>>().len() != keys.len()
        || approved.policy_epoch == 0 || !valid_bucket(approved.bucket)
    {
        return Err(SignedRootError::InvalidApproval);
    }
    let previous = match (root.generation(), approved.previous_trusted_root) {
        (1, None) => None,
        (1, Some(_)) => return Err(SignedRootError::WrongAncestry),
        (_, None) => return Err(SignedRootError::MissingPredecessor),
        (_, Some(old)) => {
            if !root.is_direct_successor_of(approved.keyspace, old) {
                return Err(SignedRootError::WrongAncestry);
            }
            Some(old.digest(approved.keyspace).map_err(|_| SignedRootError::WrongAncestry)?)
        }
    };
    let root_digest = root.digest(approved.keyspace).map_err(|_| SignedRootError::WrongRoot)?;
    if env.payload_type != ROOT_PAYLOAD_TYPE {
        return Err(SignedRootError::InvalidType);
    }
    if env.payload.is_empty() || env.payload.len() > MAX_PAYLOAD_BYTES
        || env.signatures.is_empty() || env.signatures.len() > MAX_SIGNATURES
    {
        return Err(SignedRootError::InvalidEnvelope);
    }
    for signature in env.signatures {
        if signature.sig.len() != 64 || signature.keyid.is_some_and(|v| v.len() > 128) {
            return Err(SignedRootError::InvalidSignature);
        }
    }
    let signed_bytes = pae(env.payload_type.as_bytes(), env.payload);
    let mut verified = 0;
    // Count distinct AUTHORIZED keys rather than signatures/keyid hints.
    for key in keys {
        let verifier = UnparsedPublicKey::new(&ED25519, key);
        if env.signatures.iter().any(|s| verifier.verify(&signed_bytes, s.sig).is_ok()) {
            verified += 1;
        }
    }
    if verified < policy.quorum { return Err(SignedRootError::MissingQuorum); }
    // Exact authenticated payload bytes are parsed ONCE, after the signature.
    let signed: AttestationV1 = serde_json::from_slice(env.payload)
        .map_err(|_| SignedRootError::InvalidSignedPayload)?;
    if signed.schema != 1 { return Err(SignedRootError::InvalidSignedPayload); }
    if signed.provider != "yandex-object-storage"
        || signed.endpoint != YANDEX_ENDPOINT
        || signed.region != YANDEX_REGION
        || !signed.force_path_style
        || signed.bucket != approved.bucket
        || signed.current_head_key != approved.keyspace.current_head_key()
        || signed.vault_id != approved.keyspace.vault_id()
        || signed.policy_epoch != approved.policy_epoch
    {
        return Err(SignedRootError::WrongScope);
    }
    if signed.root_blake3 != root_digest
        || signed.generation != root.generation()
        || signed.previous_root_blake3 != previous
    {
        return Err(SignedRootError::WrongRoot);
    }
    Ok(SignedRootObservation {
        root_blake3: root_digest,
        generation: root.generation(),
        verifying_distinct_keys: verified,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::OperationId;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::{json, Value};

    fn key(n: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[n; 32]).unwrap()
    }
    fn pk(k: &Ed25519KeyPair) -> [u8; 32] {
        k.public_key().as_ref().try_into().unwrap()
    }
    fn genesis(ns: &S3Keyspace) -> S3PackedHeadV2 {
        S3PackedHeadV2::new(ns, 1, OperationId::new(),
            &blake3::hash(b"m1").to_hex().to_string(), OperationId::new(),
            &blake3::hash(b"i1").to_hex().to_string(), None).unwrap()
    }
    fn next(ns: &S3Keyspace, old: &S3PackedHeadV2) -> S3PackedHeadV2 {
        S3PackedHeadV2::new(ns, old.generation() + 1, OperationId::new(),
            &blake3::hash(b"m2").to_hex().to_string(), OperationId::new(),
            &blake3::hash(b"i2").to_hex().to_string(),
            Some(old.digest(ns).unwrap())).unwrap()
    }
    fn namespace() -> S3Keyspace {
        S3Keyspace::new("akula/lab", VaultId::new()).unwrap()
    }
    fn scope<'a>(ns: &'a S3Keyspace, prev: Option<&'a S3PackedHeadV2>)
        -> ApprovedV2RootScope<'a>
    {
        ApprovedV2RootScope { bucket: "akula-test", keyspace: ns,
            policy_epoch: 2, previous_trusted_root: prev }
    }
    fn body(root: &S3PackedHeadV2, a: &ApprovedV2RootScope<'_>) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema": 1,
            "provider": "yandex-object-storage",
            "endpoint": YANDEX_ENDPOINT,
            "region": YANDEX_REGION,
            "force_path_style": true,
            "bucket": a.bucket,
            "current_head_key": a.keyspace.current_head_key(),
            "vault_id": a.keyspace.vault_id(),
            "generation": root.generation(),
            "root_blake3": root.digest(a.keyspace).unwrap(),
            "previous_root_blake3": a.previous_trusted_root
                .map(|p| p.digest(a.keyspace).unwrap()),
            "policy_epoch": a.policy_epoch,
        })).unwrap()
    }
    fn signatures<'a>(payload: &[u8], keys: &'a [&Ed25519KeyPair])
        -> Vec<Vec<u8>>
    {
        let signed = pae(ROOT_PAYLOAD_TYPE.as_bytes(), payload);
        keys.iter().map(|k| k.sign(&signed).as_ref().to_vec()).collect()
    }
    fn prove(
        payload: &[u8], sigs: &[Vec<u8>], root: &S3PackedHeadV2,
        scope: &ApprovedV2RootScope<'_>, approved: &[[u8; 32]], quorum: usize,
    ) -> Result<SignedRootObservation, SignedRootError> {
        let sigs: Vec<_> = sigs.iter().map(|s| DecodedSignature {
            sig: s, keyid: Some("untrusted-hint"),
        }).collect();
        verify_signed_root(&DecodedDsseEnvelope {
            payload_type: ROOT_PAYLOAD_TYPE, payload, signatures: &sigs,
        }, root, scope, &ApprovedPublisherKeys {
            public_keys: approved, quorum,
        })
    }

    #[test]
    fn pae_bytes_match_independent_dsse_vector() {
        assert_eq!(pae(b"abc", b"hello"), b"DSSEv1 3 abc 5 hello");
        assert_eq!(pae(b"", b""), b"DSSEv1 0  0 ");
        assert_eq!(pae(b"abc", &[0, 255]), b"DSSEv1 3 abc 2 \x00\xff");
    }

    #[test]
    fn genesis_is_bound_to_external_key_and_root() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let signer = key(1); let b = body(&root, &a);
        let sigs = signatures(&b, &[&signer]);
        let ok = prove(&b, &sigs, &root, &a, &[pk(&signer)], 1).unwrap();
        assert_eq!(ok.generation, 1);
        assert_eq!(ok.root_blake3, root.digest(&ns).unwrap());
        assert_eq!(ok.verifying_distinct_keys, 1);
    }

    #[test]
    fn two_distinct_keys_required_for_two_of_two() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let (alice, bob) = (key(1), key(2));
        let b = body(&root, &a); let keys = [pk(&alice), pk(&bob)];
        assert_eq!(prove(&b, &signatures(&b, &[&alice, &bob]), &root, &a, &keys, 2)
            .unwrap().verifying_distinct_keys, 2);
        assert_eq!(prove(&b, &signatures(&b, &[&alice, &alice]), &root, &a,
            &keys, 2), Err(SignedRootError::MissingQuorum));
        assert_eq!(prove(&b, &signatures(&b, &[&alice]), &root, &a,
            &[keys[0], keys[0]], 2), Err(SignedRootError::InvalidApproval));
    }

    #[test]
    fn wrong_key_tampered_payload_and_signature_rejected() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let (alice, wrong) = (key(1), key(4));
        let b = body(&root, &a); let sigs = signatures(&b, &[&alice]);
        assert_eq!(prove(&b, &sigs, &root, &a, &[pk(&wrong)], 1),
            Err(SignedRootError::MissingQuorum));
        assert_eq!(prove(b"{}", &sigs, &root, &a, &[pk(&alice)], 1),
            Err(SignedRootError::MissingQuorum));
        assert_eq!(prove(&b, &[vec![0u8; 64]], &root, &a, &[pk(&alice)], 1),
            Err(SignedRootError::MissingQuorum));
    }

    #[test]
    fn type_confusion_and_oversize_fail_closed() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let alice = key(1); let b = body(&root, &a);
        let sigs = signatures(&b, &[&alice]);
        let decoded = [DecodedSignature { sig: &sigs[0], keyid: None }];
        assert_eq!(verify_signed_root(&DecodedDsseEnvelope {
            payload_type: "application/json", payload: &b, signatures: &decoded,
        }, &root, &a, &ApprovedPublisherKeys {
            public_keys: &[pk(&alice)], quorum: 1,
        }), Err(SignedRootError::InvalidType));
        assert_eq!(prove(&vec![1u8; MAX_PAYLOAD_BYTES + 1], &sigs, &root,
            &a, &[pk(&alice)], 1), Err(SignedRootError::InvalidEnvelope));
    }

    #[test]
    fn signer_not_allowed_to_change_bucket_vault_or_policy() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let alice = key(1); let original = body(&root, &a);
        for (field, value) in [
            ("bucket", json!("other-bucket")),
            ("endpoint", json!("https://attacker.invalid")),
            ("region", json!("us-east-1")),
            ("policy_epoch", json!(1)),
            ("current_head_key", json!("other/vault/refs/current.json")),
            ("vault_id", json!(VaultId::new())),
        ] {
            let mut v: Value = serde_json::from_slice(&original).unwrap();
            v[field] = value;
            let b = serde_json::to_vec(&v).unwrap();
            assert_eq!(prove(&b, &signatures(&b, &[&alice]), &root,
                &a, &[pk(&alice)], 1), Err(SignedRootError::WrongScope));
        }
    }

    #[test]
    fn swapped_root_or_generation_and_signed_unknown_fields_fail() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let alice = key(1); let b = body(&root, &a);
        let alternative = genesis(&ns);
        assert_eq!(prove(&b, &signatures(&b, &[&alice]), &alternative, &a,
            &[pk(&alice)], 1), Err(SignedRootError::WrongRoot));
        let mut v: Value = serde_json::from_slice(&b).unwrap();
        v["generation"] = json!(7);
        let changed = serde_json::to_vec(&v).unwrap();
        assert_eq!(prove(&changed, &signatures(&changed, &[&alice]), &root,
            &a, &[pk(&alice)], 1), Err(SignedRootError::WrongRoot));
        v = serde_json::from_slice(&b).unwrap();
        v["unknown_authority"] = json!("must reject");
        let unknown = serde_json::to_vec(&v).unwrap();
        assert_eq!(prove(&unknown, &signatures(&unknown, &[&alice]), &root,
            &a, &[pk(&alice)], 1), Err(SignedRootError::InvalidSignedPayload));
    }

    #[test]
    fn previous_generation_must_be_explicit_and_direct() {
        let ns = namespace(); let root = genesis(&ns);
        let second = next(&ns, &root); let alice = key(1);
        let missing = scope(&ns, None); let a = scope(&ns, Some(&root));
        let b = body(&second, &a);
        assert_eq!(prove(&b, &signatures(&b, &[&alice]), &second,
            &missing, &[pk(&alice)], 1), Err(SignedRootError::MissingPredecessor));
        assert_eq!(prove(&b, &signatures(&b, &[&alice]), &second,
            &a, &[pk(&alice)], 1).unwrap().generation, 2);
        let rollback = scope(&ns, Some(&second));
        assert_eq!(prove(&body(&root, &missing),
            &signatures(&body(&root, &missing), &[&alice]), &root,
            &rollback, &[pk(&alice)], 1), Err(SignedRootError::WrongAncestry));
    }

    #[test]
    fn forged_keyid_is_never_authority() {
        let ns = namespace(); let root = genesis(&ns); let a = scope(&ns, None);
        let (alice, impostor) = (key(1), key(9)); let b = body(&root, &a);
        let sigs = signatures(&b, &[&alice]);
        let decoded = [DecodedSignature {
            sig: &sigs[0], keyid: Some("owner-key"),
        }];
        let env = DecodedDsseEnvelope {
            payload_type: ROOT_PAYLOAD_TYPE, payload: &b, signatures: &decoded,
        };
        assert!(verify_signed_root(&env, &root, &a, &ApprovedPublisherKeys {
            public_keys: &[pk(&alice)], quorum: 1,
        }).is_ok());
        assert_eq!(verify_signed_root(&env, &root, &a, &ApprovedPublisherKeys {
            public_keys: &[pk(&impostor)], quorum: 1,
        }), Err(SignedRootError::MissingQuorum));
    }
}
