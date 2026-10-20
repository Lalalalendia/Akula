//! YC-S3-T01d E3b: explicitly approved, bounded read-only V2 joining probe.
//!
//! E3a verifies an owner-signed genesis invitation from an independently
//! authenticated channel. This module *also* requires full existing pinned V2
//! root/manifest/index/pack inspection and re-checks the root ETag afterward.
//! It does not save a trust anchor, create users, mint credentials, overwrite
//! notes, grant File Gate permission, PUT/CAS/DELETE, or enable Sync.
//! The observed S3 ETag is a concurrency token, NEVER an owner signature.
//! The operator must explicitly provide credentials and call this operation;
//! nothing is scheduled or discovered automatically.

use super::{
    review_offline_join_genesis, AwsS3ProbeClient, DecodedDsseEnvelope,
    IndependentlyApprovedJoin, JoinGenesisError, JoinGenesisReview,
    PublisherPolicyV1, S3PackedHeadV2, S3ConnectionProfile,
    S3PinnedV2ReadError, S3PinnedV2ReadPreview, S3RemoteAnchorScope,
    S3TransportError, S3Keyspace, MAX_PACK_V2_ROOT_BYTES,
};
use super::s3_signed_root_paev1::{YANDEX_ENDPOINT, YANDEX_REGION};

#[derive(Debug)]
pub enum PinnedJoinError {
    Join(JoinGenesisError),
    WrongScope,
    WrongRemoteGenesis,
    InvalidEtag,
    UnverifiedCompleteRead,
    Contract,
    Transport(S3TransportError),
    Pinned(S3PinnedV2ReadError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinnedJoinReadOnlyObservation {
    pub owner_review: JoinGenesisReview,
    pub verified_revisions: usize,
    pub verified_packs: usize,
    pub verified_pack_bytes: u64,
    pub requires_explicit_file_gate_review: bool,
}

fn checked_scope(
    scope: &S3RemoteAnchorScope,
    approved: &IndependentlyApprovedJoin<'_>,
) -> Result<(), PinnedJoinError> {
    // The E3a signed invitation explicitly promises Yandex. Checking only
    // the bucket/keyspace would permit caller-supplied MinIO/foreign HTTPS
    // endpoint credentials to masquerade as the approved Yandex provider.
    let expected_profile = S3ConnectionProfile {
        bucket: approved.bucket.to_owned(),
        region: YANDEX_REGION.to_owned(),
        endpoint: Some(YANDEX_ENDPOINT.to_owned()),
        force_path_style: true,
    };
    if scope.keyspace() != approved.keyspace
        || scope.policy_bucket() != approved.bucket
        || !scope.matches_connection(&expected_profile, approved.keyspace)
    {
        return Err(PinnedJoinError::WrongScope);
    }
    Ok(())
}

fn checked_etag(etag: &str) -> bool {
    !etag.is_empty() && etag.len() <= 256
        && etag.bytes().all(|b| !b.is_ascii_control())
}

/// Combine the independently approved signed join with a PREVIOUSLY
/// FULLY VERIFIED pinned V2 receipt, minted only by the existing read-only
/// client. This is a pure consistency check, never an enrollment grant.
pub fn inspect_pinned_join_receipt(
    scope: &S3RemoteAnchorScope,
    observed_root: &S3PackedHeadV2,
    observed_policy: &PublisherPolicyV1,
    approved: &IndependentlyApprovedJoin<'_>,
    owner_offer: &DecodedDsseEnvelope<'_>,
    root_etag: &str,
    completed_read: &S3PinnedV2ReadPreview,
) -> Result<PinnedJoinReadOnlyObservation, PinnedJoinError> {
    checked_scope(scope, approved)?;
    let owner_review = review_offline_join_genesis(
        owner_offer, observed_root, observed_policy, approved,
    ).map_err(PinnedJoinError::Join)?;
    if !checked_etag(root_etag) {
        return Err(PinnedJoinError::InvalidEtag);
    }
    if !completed_read.attests_exact_root(scope, observed_root, root_etag) {
        return Err(PinnedJoinError::UnverifiedCompleteRead);
    }
    Ok(PinnedJoinReadOnlyObservation {
        owner_review,
        verified_revisions: completed_read.verified_revisions,
        verified_packs: completed_read.verified_packs,
        verified_pack_bytes: completed_read.verified_pack_bytes,
        requires_explicit_file_gate_review: true,
    })
}

/// Optional explicitly invoked *read-only* Yandex S3 probe, with no default
/// credentials or background worker in this module. It first validates E3a,
/// then reads exactly the approved V2 current-head key to OBSERVE its ETag.
/// The ETag is not authenticated out of band and cannot select a new root:
/// exact independently approved bytes MUST match, before and after the
/// existing full bounded pack/manifest/index inspector.
pub async fn preview_yandex_join_genesis_read_only(
    client: &AwsS3ProbeClient,
    scope: &S3RemoteAnchorScope,
    expected_root: &S3PackedHeadV2,
    expected_policy: &PublisherPolicyV1,
    approved: &IndependentlyApprovedJoin<'_>,
    owner_offer: &DecodedDsseEnvelope<'_>,
) -> Result<PinnedJoinReadOnlyObservation, PinnedJoinError> {
    // Do the immutable identity+signature checks BEFORE network, not by
    // trusting arbitrary owner keys, namespace, or root bytes from S3.
    checked_scope(scope, approved)?;
    review_offline_join_genesis(
        owner_offer, expected_root, expected_policy, approved,
    ).map_err(PinnedJoinError::Join)?;

    if !scope.matches_connection(client.bound_profile(), client.bound_keyspace()) {
        return Err(PinnedJoinError::WrongScope);
    }
    let keyspace: &S3Keyspace = scope.keyspace();
    let expected_bytes = expected_root.encode(keyspace)
        .map_err(|_| PinnedJoinError::Contract)?;
    let key = keyspace.current_head_key();
    let initial = client.get_bounded(&key, MAX_PACK_V2_ROOT_BYTES as u64)
        .await.map_err(PinnedJoinError::Transport)?;
    if initial.bytes != expected_bytes {
        return Err(PinnedJoinError::WrongRemoteGenesis);
    }
    let etag = initial.object.etag;
    if !checked_etag(&etag) {
        return Err(PinnedJoinError::InvalidEtag);
    }
    // Fully bound/verified remote GET of root, immutable manifest, shards,
    // index and packs; the client detects concurrent current-head changes.
    let read = client.inspect_explicitly_pinned_v2_read_only(
        scope, expected_root, &etag, None,
    ).await.map_err(PinnedJoinError::Pinned)?;
    inspect_pinned_join_receipt(
        scope, expected_root, expected_policy, approved, owner_offer,
        &etag, &read,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{
        DecodedSignature, OperationId, VaultId,
    };
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use serde_json::{json, Value};

    const CHALLENGE: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn owner(seed: u8) -> Ed25519KeyPair {
        Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
    }
    fn public_key(s: &Ed25519KeyPair) -> [u8; 32] {
        s.public_key().as_ref().try_into().unwrap()
    }

    struct Fixture {
        ns: S3Keyspace,
        root: S3PackedHeadV2,
        policy: PublisherPolicyV1,
        scope: S3RemoteAnchorScope,
        profile: S3ConnectionProfile,
        owner: Ed25519KeyPair,
        root_pin: String,
        policy_pin: String,
        keys: Vec<[u8; 32]>,
    }
    impl Fixture {
        fn new() -> Self {
            let ns = S3Keyspace::new("akula/e3b-private", VaultId::new()).unwrap();
            let root = S3PackedHeadV2::new(
                &ns, 1, OperationId::new(),
                &blake3::hash(b"e3b-manifest").to_hex().to_string(),
                OperationId::new(),
                &blake3::hash(b"e3b-index").to_hex().to_string(),
                None,
            ).unwrap();
            let signing_owner = owner(7);
            let keys = vec![public_key(&signing_owner)];
            let profile = S3ConnectionProfile {
                bucket: "akula-private-e3b".to_owned(),
                region: "ru-central1".to_owned(),
                endpoint: Some("https://storage.yandexcloud.net".to_owned()),
                force_path_style: true,
            };
            let scope = S3RemoteAnchorScope::new(
                "e3b-isolated-local-profile",
                "out-of-band-account-binding-required",
                &profile, &ns,
            ).unwrap();
            let policy_bytes = serde_json::to_vec(&json!({
                "schema": 1,
                "epoch": 1,
                "provider": "yandex-object-storage",
                "endpoint": "https://storage.yandexcloud.net",
                "region": "ru-central1",
                "force_path_style": true,
                "bucket": profile.bucket,
                "vault_id": ns.vault_id(),
                "current_head_key": ns.current_head_key(),
                "owner_keys": &keys,
                "owner_quorum": 1,
                "publisher_keys": [public_key(&owner(99))],
                "publisher_quorum": 1,
                "previous_policy_blake3": null,
            })).unwrap();
            let policy = PublisherPolicyV1::parse_for_review(&policy_bytes).unwrap();
            let policy_pin = policy.canonical_digest().unwrap();
            let root_pin = root.digest(&ns).unwrap();
            Self { ns, root, policy, scope, profile, owner: signing_owner,
                root_pin, policy_pin, keys }
        }

        fn approved(&self) -> IndependentlyApprovedJoin<'_> {
            IndependentlyApprovedJoin {
                bucket: "akula-private-e3b",
                keyspace: &self.ns,
                pinned_genesis_root_blake3: &self.root_pin,
                pinned_genesis_policy_blake3: &self.policy_pin,
                trusted_owner_keys: &self.keys,
                owner_quorum: 1,
                recipient_challenge_hex: CHALLENGE,
            }
        }

        fn offer(&self) -> Vec<u8> {
            serde_json::to_vec(&json!({
                "schema": 1,
                "purpose": "review-only-join-genesis",
                "provider": "yandex-object-storage",
                "endpoint": "https://storage.yandexcloud.net",
                "region": "ru-central1",
                "force_path_style": true,
                "bucket": "akula-private-e3b",
                "current_head_key": self.ns.current_head_key(),
                "vault_id": self.ns.vault_id(),
                "root_generation": 1,
                "root_blake3": self.root_pin,
                "policy_epoch": 1,
                "policy_blake3": self.policy_pin,
                "recipient_challenge_hex": CHALLENGE,
            })).unwrap()
        }
        fn verified(&self, root: &S3PackedHeadV2, etag: &str) -> S3PinnedV2ReadPreview {
            S3PinnedV2ReadPreview::for_trust_anchor_test(&self.scope, root, etag)
        }
        fn inspect(
            &self, signed: &[u8],
            sig: &[u8],
            pinned: &S3PinnedV2ReadPreview,
            etag: &str,
            scope: &S3RemoteAnchorScope,
            root: &S3PackedHeadV2,
        ) -> Result<PinnedJoinReadOnlyObservation, PinnedJoinError> {
            let signatures = [DecodedSignature { sig, keyid: Some("untrusted-keyid") }];
            inspect_pinned_join_receipt(
                scope, root, &self.policy, &self.approved(),
                &DecodedDsseEnvelope {
                    payload_type: super::super::JOIN_GENESIS_TYPE,
                    payload: signed, signatures: &signatures,
                }, etag, pinned,
            )
        }
    }
    fn signed_fixture(f: &Fixture) -> (Vec<u8>, Vec<u8>) {
        let bytes = f.offer();
        let sig = f.owner.sign(
            &super::super::s3_signed_root_paev1::pae(
                super::super::JOIN_GENESIS_TYPE.as_bytes(), &bytes,
            )
        ).as_ref().to_vec();
        (bytes, sig)
    }

    #[test]
    fn independently_signed_and_pinned_full_genesis_produces_readonly_observation() {
        let f = Fixture::new();
        let (body, signature) = signed_fixture(&f);
        let etag = r#""provider-etag-is-not-owner-signature""#;
        let checked = f.inspect(
            &body, &signature, &f.verified(&f.root, etag),
            etag, &f.scope, &f.root,
        ).unwrap();
        assert_eq!(checked.owner_review.root_blake3, f.root_pin);
        assert_eq!(checked.owner_review.policy_blake3, f.policy_pin);
        assert_eq!(checked.verified_revisions, 1);
        assert!(checked.requires_explicit_file_gate_review);
    }

    #[test]
    fn server_etag_missing_or_replaced_cannot_forge_complete_read() {
        let f = Fixture::new();
        let (body, signature) = signed_fixture(&f);
        let pinned = f.verified(&f.root, "etag-a");
        for etag in ["", "\n", "etag-b"] {
            assert!(f.inspect(
                &body, &signature, &pinned, etag, &f.scope, &f.root,
            ).is_err());
        }
    }

    #[test]
    fn wrong_remote_profile_bucket_and_vault_namespace_are_held() {
        let f = Fixture::new();
        let (body, signature) = signed_fixture(&f);
        let mut different = f.profile.clone();
        different.bucket = "other-e3b-bucket".into();
        let foreign_bucket = S3RemoteAnchorScope::new(
            f.scope.profile_id(), "out-of-band-account-binding-required",
            &different, &f.ns,
        ).unwrap();
        assert!(matches!(
            f.inspect(&body, &signature, &f.verified(&f.root, "e"),
                "e", &foreign_bucket, &f.root),
            Err(PinnedJoinError::WrongScope)
        ));
        let different_ns = S3Keyspace::new("other", VaultId::new()).unwrap();
        let foreign = S3RemoteAnchorScope::new(
            f.scope.profile_id(), "out-of-band-account-binding-required",
            &f.profile, &different_ns,
        ).unwrap();
        assert!(matches!(
            f.inspect(&body, &signature, &f.verified(&f.root, "e"),
                "e", &foreign, &f.root),
            Err(PinnedJoinError::WrongScope)
        ));
    }

    #[test]
    fn complete_read_for_another_genesis_is_not_transferable() {
        let f = Fixture::new();
        let (body, signature) = signed_fixture(&f);
        let other = S3PackedHeadV2::new(
            &f.ns, 1, OperationId::new(),
            &blake3::hash(b"tampered").to_hex().to_string(),
            OperationId::new(),
            &blake3::hash(b"also-tampered").to_hex().to_string(), None,
        ).unwrap();
        let other_pinned = f.verified(&other, "e");
        assert!(matches!(
            f.inspect(&body, &signature, &other_pinned, "e", &f.scope, &f.root),
            Err(PinnedJoinError::UnverifiedCompleteRead)
        ));
        assert!(matches!(
            f.inspect(&body, &signature, &other_pinned, "e", &f.scope, &other),
            Err(PinnedJoinError::Join(JoinGenesisError::WrongIndependentPin))
        ));
    }

    #[test]
    fn valid_full_read_alone_does_not_authenticate_owner() {
        let f = Fixture::new();
        let (body, _) = signed_fixture(&f);
        let attacker = owner(44);
        let forged = attacker.sign(
            &super::super::s3_signed_root_paev1::pae(
                super::super::JOIN_GENESIS_TYPE.as_bytes(), &body,
            )
        ).as_ref().to_vec();
        assert!(matches!(
            f.inspect(&body, &forged, &f.verified(&f.root, "e"),
                "e", &f.scope, &f.root),
            Err(PinnedJoinError::Join(JoinGenesisError::MissingOwnerQuorum))
        ));
    }

    #[test]
    fn right_owner_signature_without_complete_pinned_read_is_not_enough() {
        let f = Fixture::new();
        let (body, sig) = signed_fixture(&f);
        let mut partial = f.verified(&f.root, "e");
        partial.requires_explicit_file_gate_review = false;
        assert!(matches!(
            f.inspect(&body, &sig, &partial, "e", &f.scope, &f.root),
            Err(PinnedJoinError::UnverifiedCompleteRead)
        ));
        let other = f.verified(&f.root, "changed-etag");
        assert!(matches!(
            f.inspect(&body, &sig, &other, "e", &f.scope, &f.root),
            Err(PinnedJoinError::UnverifiedCompleteRead)
        ));
    }

    #[test]
    fn joined_root_can_never_be_implicitly_advanced_from_remote() {
        let f = Fixture::new();
        let (body, sig) = signed_fixture(&f);
        let newer = S3PackedHeadV2::new(
            &f.ns, 2, OperationId::new(),
            &blake3::hash(b"newer-m").to_hex().to_string(),
            OperationId::new(),
            &blake3::hash(b"newer-i").to_hex().to_string(),
            Some(f.root_pin.clone()),
        ).unwrap();
        assert!(matches!(
            f.inspect(&body, &sig, &f.verified(&newer, "e"),
                "e", &f.scope, &newer),
            Err(PinnedJoinError::Join(JoinGenesisError::NotGenesis))
        ));
    }

    #[test]
    fn signed_approval_cannot_be_replayed_against_changed_recipient_challenge() {
        let f = Fixture::new();
        let (mut body, _) = signed_fixture(&f);
        let mut value: Value = serde_json::from_slice(&body).unwrap();
        value["recipient_challenge_hex"] = json!("f".repeat(64));
        body = serde_json::to_vec(&value).unwrap();
        // Invalid signature would be first hold, since challenge is signed.
        let signature = f.owner.sign(
            &super::super::s3_signed_root_paev1::pae(
                super::super::JOIN_GENESIS_TYPE.as_bytes(), &body,
            )
        ).as_ref().to_vec();
        assert!(matches!(
            f.inspect(&body, &signature, &f.verified(&f.root, "e"),
                "e", &f.scope, &f.root),
            Err(PinnedJoinError::Join(JoinGenesisError::WrongChallenge))
        ));
    }
}
