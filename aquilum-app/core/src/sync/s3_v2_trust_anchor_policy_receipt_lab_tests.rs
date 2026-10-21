use super::*;
use crate::app_core::{Core, CoreEvent};
use crate::sync::{
    DecodedSignature, OperationId, S3ConnectionProfile, S3Keyspace, VaultId,
    POLICY_PAYLOAD_TYPE,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc};

struct Fixture {
    _tmp: tempfile::TempDir,
    core: Arc<Core>,
    vault: PathBuf,
    appdata: PathBuf,
    ns: S3Keyspace,
    scope: S3RemoteAnchorScope,
    profile: S3ConnectionProfile,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("my-markdown-vault");
        let appdata = tmp.path().join("private-appdata");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::create_dir_all(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
        let ns = S3Keyspace::new("akula/publisher-policy", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-policy-test".into(),
            region: "ru-central1".into(),
            endpoint: Some("https://storage.yandexcloud.net".into()),
            force_path_style: true,
        };
        let scope = S3RemoteAnchorScope::new(
            "e2b-local-profile", "independently-approved-account-binding",
            &profile, &ns,
        ).unwrap();
        Self { _tmp: tmp, core, vault, appdata, ns, scope, profile }
    }
    fn path(&self) -> PathBuf { self.appdata.join("v2-trust.sqlite") }
    fn root(&self) -> S3PackedHeadV2 {
        S3PackedHeadV2::new(&self.ns, 1, OperationId::new(),
            &blake3::hash(b"e2b-genesis-manifest").to_hex().to_string(),
            OperationId::new(),
            &blake3::hash(b"e2b-genesis-index").to_hex().to_string(),
            None).unwrap()
    }
    fn open(&self) -> S3V2TrustedAnchorJournal {
        let admission = RegisteredReadAdmission::open(&self.core, &self.vault).unwrap();
        S3V2TrustedAnchorJournal::open(&admission, &self.path()).unwrap()
    }
    fn with_pinned_root(&self) -> S3V2TrustedAnchorJournal {
        let mut journal = self.open();
        let root = self.root();
        let fp = root.digest(&self.ns).unwrap();
        let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(
            &self.scope, &root, "etag-e2b-genesis",
        );
        journal.enroll_verified_genesis(
            &self.scope, &fp, &root, "etag-e2b-genesis", &pinned,
        ).unwrap();
        journal
    }
}

fn signer(seed: u8) -> Ed25519KeyPair {
    Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap()
}
fn pk(key: &Ed25519KeyPair) -> [u8; 32] {
    key.public_key().as_ref().try_into().unwrap()
}
fn first_policy(f: &Fixture, owner: &Ed25519KeyPair) -> (PublisherPolicyV1, Vec<u8>) {
    let json = json!({
        "schema": 1,
        "epoch": 1,
        "provider": "yandex-object-storage",
        "endpoint": "https://storage.yandexcloud.net",
        "region": "ru-central1",
        "force_path_style": true,
        "bucket": f.profile.bucket,
        "vault_id": f.ns.vault_id(),
        "current_head_key": f.ns.current_head_key(),
        "owner_keys": [pk(owner)],
        "owner_quorum": 1,
        "publisher_keys": [pk(&signer(99))],
        "publisher_quorum": 1,
        "previous_policy_blake3": null,
    });
    let parsed = PublisherPolicyV1::parse_for_review(&serde_json::to_vec(&json).unwrap()).unwrap();
    let bytes = serde_json::to_vec(&parsed).unwrap();
    (parsed, bytes)
}
fn next_policy(
    old: &PublisherPolicyV1, new_owners: &[[u8; 32]],
    new_quorum: usize, publisher_keys: &[[u8; 32]],
) -> Vec<u8> {
    let mut json = serde_json::to_value(old).unwrap();
    json["epoch"] = json!(old.epoch() + 1);
    json["previous_policy_blake3"] = json!(old.canonical_digest().unwrap());
    json["owner_keys"] = json!(new_owners);
    json["owner_quorum"] = json!(new_quorum);
    json["publisher_keys"] = json!(publisher_keys);
    json["publisher_quorum"] = json!(if publisher_keys.is_empty() { 0 } else { 1 });
    let parsed = PublisherPolicyV1::parse_for_review(&serde_json::to_vec(&json).unwrap()).unwrap();
    serde_json::to_vec(&parsed).unwrap()
}
fn record(
    journal: &mut S3V2TrustedAnchorJournal,
    scope: &S3RemoteAnchorScope,
    bytes: &[u8],
    signers: &[&Ed25519KeyPair],
) -> Result<PolicyReceiptOutcome, V2PolicyReceiptError> {
    let pae = super::super::super::s3_signed_root_paev1::pae(
        POLICY_PAYLOAD_TYPE.as_bytes(), bytes,
    );
    let raw: Vec<_> = signers.iter().map(|key| key.sign(&pae).as_ref().to_vec()).collect();
    let signatures: Vec<_> = raw.iter().map(|sig| DecodedSignature {
        sig, keyid: Some("untrusted-label-does-not-grant-owner-access"),
    }).collect();
    journal.record_verified_publisher_transition(scope, &DecodedDsseEnvelope {
        payload_type: POLICY_PAYLOAD_TYPE,
        payload: bytes,
        signatures: &signatures,
    })
}

#[test]
fn no_trusted_root_means_no_policy_enrollment_or_implicit_tofu() {
    let f = Fixture::new();
    let mut journal = f.open();
    let owner = signer(1);
    let (policy, bytes) = first_policy(&f, &owner);
    let digest = policy.canonical_digest().unwrap();
    assert!(matches!(
        journal.enroll_externally_approved_genesis_policy(&f.scope, &digest, &bytes),
        Err(V2PolicyReceiptError::NoAnchoredRoot),
    ));
    assert!(matches!(
        journal.load_publisher_policy(&f.scope),
        Err(V2PolicyReceiptError::NoAnchoredRoot),
    ));
}

#[test]
fn pinned_root_and_independent_policy_pin_survive_restart_without_s3_writes() {
    let f = Fixture::new();
    let mut journal = f.with_pinned_root();
    let (policy, bytes) = first_policy(&f, &signer(1));
    let digest = policy.canonical_digest().unwrap();
    assert!(matches!(
        journal.enroll_externally_approved_genesis_policy(&f.scope, &"f".repeat(64), &bytes),
        Err(V2PolicyReceiptError::InvalidExternalApproval)
    ));
    assert!(journal.load_publisher_policy(&f.scope).unwrap().is_none());
    assert_eq!(journal.enroll_externally_approved_genesis_policy(
        &f.scope, &digest, &bytes,
    ).unwrap(), PolicyReceiptOutcome::Inserted);
    assert_eq!(journal.enroll_externally_approved_genesis_policy(
        &f.scope, &digest, &bytes,
    ).unwrap(), PolicyReceiptOutcome::Unchanged);
    assert_eq!(journal.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), digest);
    drop(journal);
    let reopened = f.open();
    let saved = reopened.load_publisher_policy(&f.scope).unwrap().unwrap();
    assert_eq!(saved.policy().epoch(), 1);
    assert_eq!(saved.digest(), digest);
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root().generation(), 1);
    assert_eq!(reopened.db.query_row(
        "SELECT count(*) FROM s3_v2_trusted_roots", [], |r| r.get::<_, i64>(0),
    ).unwrap(), 1);
}

#[test]
fn owner_rotation_and_delegated_publisher_revocation_survive_restart() {
    let f = Fixture::new();
    let (old_owner, next_owner) = (signer(1), signer(2));
    let mut journal = f.with_pinned_root();
    let (old, bytes) = first_policy(&f, &old_owner);
    let pin = old.canonical_digest().unwrap();
    journal.enroll_externally_approved_genesis_policy(&f.scope, &pin, &bytes).unwrap();
    let next = next_policy(&old, &[pk(&next_owner)], 1, &[]);
    assert_eq!(record(&mut journal, &f.scope, &next, &[&old_owner, &next_owner]).unwrap(),
        PolicyReceiptOutcome::Advanced);
    assert_eq!(journal.load_publisher_policy(&f.scope).unwrap().unwrap().policy().epoch(), 2);
    drop(journal);
    let reopened = f.open();
    let snapshot = reopened.load_publisher_policy(&f.scope).unwrap().unwrap();
    let p = serde_json::to_value(snapshot.policy()).unwrap();
    assert_eq!(p["publisher_keys"].as_array().unwrap().len(), 0);
    assert_eq!(p["publisher_quorum"], 0);
    assert_eq!(p["owner_keys"], json!([pk(&next_owner)]));
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root().generation(), 1);
}

#[test]
fn replay_and_same_generation_forks_never_advance_a_persisted_policy() {
    let f = Fixture::new();
    let owner = signer(1);
    let mut first_handle = f.with_pinned_root();
    let (old, bytes) = first_policy(&f, &owner);
    let pin = old.canonical_digest().unwrap();
    first_handle.enroll_externally_approved_genesis_policy(&f.scope, &pin, &bytes).unwrap();
    let mut second_handle = f.open();
    let fork_a = next_policy(&old, &[pk(&owner)], 1, &[]);
    let fork_b = next_policy(&old, &[pk(&owner)], 1, &[pk(&signer(98))]);
    assert_eq!(record(&mut first_handle, &f.scope, &fork_a, &[&owner]).unwrap(),
        PolicyReceiptOutcome::Advanced);
    assert!(matches!(
        record(&mut second_handle, &f.scope, &fork_b, &[&owner]),
        Err(V2PolicyReceiptError::InvalidTransition(PolicyLabError::InvalidTransition))
    ));
    assert!(matches!(
        record(&mut first_handle, &f.scope, &fork_a, &[&owner]),
        Err(V2PolicyReceiptError::InvalidTransition(PolicyLabError::InvalidTransition))
    ));
    let stored = second_handle.load_publisher_policy(&f.scope).unwrap().unwrap();
    assert_eq!(stored.digest(), PublisherPolicyV1::parse_for_review(&fork_a)
        .unwrap().canonical_digest().unwrap());
    assert_eq!(stored.policy().epoch(), 2);
}

#[test]
fn unsigned_forged_publisher_or_incomplete_owner_quorum_never_modify_policy() {
    let f = Fixture::new();
    let owner = signer(1);
    let mut journal = f.with_pinned_root();
    let (old, bytes) = first_policy(&f, &owner);
    let old_pin = old.canonical_digest().unwrap();
    journal.enroll_externally_approved_genesis_policy(&f.scope, &old_pin, &bytes).unwrap();
    let next_owner = signer(2);
    let next = next_policy(&old, &[pk(&next_owner)], 1, &[]);
    assert!(matches!(
        record(&mut journal, &f.scope, &next, &[&signer(99), &next_owner]),
        Err(V2PolicyReceiptError::InvalidTransition(PolicyLabError::OldOwnerQuorumMissing))
    ));
    assert!(matches!(
        record(&mut journal, &f.scope, &next, &[&owner]),
        Err(V2PolicyReceiptError::InvalidTransition(PolicyLabError::NewOwnerQuorumMissing))
    ));
    assert_eq!(journal.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), old_pin);
}

#[test]
fn corrupt_sqlite_policy_receipt_is_not_trusted_after_restart() {
    let f = Fixture::new();
    let owner = signer(1);
    let mut journal = f.with_pinned_root();
    let (policy, bytes) = first_policy(&f, &owner);
    journal.enroll_externally_approved_genesis_policy(
        &f.scope, &policy.canonical_digest().unwrap(), &bytes,
    ).unwrap();
    drop(journal);
    let external = Connection::open(f.path()).unwrap();
    external.execute("UPDATE s3_v2_trusted_roots SET policy_json=?1 WHERE profile_id=?2",
        params![b"{}".as_slice(), f.scope.profile_id()]).unwrap();
    let reopened = f.open();
    // An ordinary caller that only loads the V2 root must NOT bypass the
    // damaged policy: both APIs now fail closed on the same guarded row.
    assert!(matches!(
        reopened.load(&f.scope),
        Err(S3V2TrustAnchorError::CorruptedAnchor),
    ));
    assert!(matches!(
        reopened.load_publisher_policy(&f.scope),
        Err(V2PolicyReceiptError::Root(S3V2TrustAnchorError::CorruptedAnchor)),
    ));
}

#[test]
fn same_profile_cannot_rebind_policy_to_foreign_vault_or_bucket() {
    let f = Fixture::new();
    let owner = signer(1);
    let mut journal = f.with_pinned_root();
    let (policy, bytes) = first_policy(&f, &owner);
    journal.enroll_externally_approved_genesis_policy(
        &f.scope, &policy.canonical_digest().unwrap(), &bytes,
    ).unwrap();
    let other = S3Keyspace::new("akula/publisher-policy", VaultId::new()).unwrap();
    let foreign = S3RemoteAnchorScope::new(
        f.scope.profile_id(), "independently-approved-account-binding",
        &f.profile, &other,
    ).unwrap();
    assert!(matches!(
        journal.load_publisher_policy(&foreign),
        Err(V2PolicyReceiptError::Root(S3V2TrustAnchorError::ScopeChanged))
    ));
    let other_profile = S3ConnectionProfile {
        bucket: "other-private-bucket".into(), ..f.profile.clone()
    };
    let wrong_bucket = S3RemoteAnchorScope::new(
        f.scope.profile_id(), "independently-approved-account-binding",
        &other_profile, &f.ns,
    ).unwrap();
    assert!(matches!(
        journal.load_publisher_policy(&wrong_bucket),
        Err(V2PolicyReceiptError::Root(S3V2TrustAnchorError::ScopeChanged))
    ));
}

#[test]
fn existing_anchor_schema_migration_is_idempotent_and_guarded() {
    let f = Fixture::new();
    let original = f.open();
    let (mut j, old_root) = (original, f.root());
    let preview = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &old_root, "etag-e2b",
    );
    j.enroll_verified_genesis(&f.scope, &old_root.digest(&f.ns).unwrap(),
        &old_root, "etag-e2b", &preview).unwrap();
    drop(j);
    // Reopening must NOT create new independent policy rows and must preserve
    // the root. Migration is additive and the same row remains authoritative.
    let reopened = f.open();
    assert!(reopened.load(&f.scope).unwrap().is_some());
    let columns: Vec<String> = reopened.db.prepare("PRAGMA table_info(s3_v2_trusted_roots)")
        .unwrap().query_map([], |r| r.get(1)).unwrap()
        .collect::<Result<_, _>>().unwrap();
    assert!(POLICY_COLUMNS.iter().all(|name|
        columns.iter().any(|column| column == name)));
    assert!(reopened.load_publisher_policy(&f.scope).unwrap().is_none());
    let admission = RegisteredReadAdmission::open(&f.core, &f.vault).unwrap();
    assert!(matches!(
        S3V2TrustedAnchorJournal::open(&admission, &f.vault.join("unsafe.sqlite")),
        Err(S3V2TrustAnchorError::InvalidStoreLocation),
    ));
}

fn publisher_root_signature(
    journal: &mut S3V2TrustedAnchorJournal,
    f: &Fixture,
    root: &S3PackedHeadV2,
    epoch: u64,
    publisher: &Ed25519KeyPair,
) -> Result<crate::sync::SignedRootObservation, V2PolicyReceiptError> {
    // Synthetic *in-memory* attestation; no S3 fetch, signing key storage,
    // remote writer, user note modification or SQLite trust enrollment.
    let payload = serde_json::to_vec(&json!({
        "schema": 1,
        "provider": "yandex-object-storage",
        "endpoint": "https://storage.yandexcloud.net",
        "region": "ru-central1",
        "force_path_style": true,
        "bucket": f.profile.bucket,
        "current_head_key": f.ns.current_head_key(),
        "vault_id": f.ns.vault_id(),
        "generation": root.generation(),
        "root_blake3": root.digest(&f.ns).unwrap(),
        "previous_root_blake3": null,
        "policy_epoch": epoch,
    })).unwrap();
    let signed = super::super::super::s3_signed_root_paev1::pae(
        crate::sync::ROOT_PAYLOAD_TYPE.as_bytes(), &payload,
    );
    let sig = publisher.sign(&signed).as_ref().to_vec();
    let signatures = [DecodedSignature {
        sig: &sig,
        keyid: Some("forged-keyid-not-authority"),
    }];
    journal.inspect_current_root_publisher_signature(
        &f.scope, root, &DecodedDsseEnvelope {
            payload_type: crate::sync::ROOT_PAYLOAD_TYPE,
            payload: &payload,
            signatures: &signatures,
        },
    )
}

#[test]
fn revoked_publisher_cannot_verify_pinned_root_after_restart() {
    let f = Fixture::new();
    let (owner, new_owner, old_publisher, new_publisher) =
        (signer(1), signer(2), signer(99), signer(98));
    let mut journal = f.with_pinned_root();
    let root = journal.load(&f.scope).unwrap().unwrap().root().clone();
    let (old, bytes) = first_policy(&f, &owner);
    journal.enroll_externally_approved_genesis_policy(
        &f.scope, &old.canonical_digest().unwrap(), &bytes,
    ).unwrap();
    assert!(publisher_root_signature(&mut journal, &f, &root, 1, &old_publisher).is_ok());

    // The owner transition preserves the SAME pinned V2 root but revokes
    // publisher-99 and delegates root attestations to publisher-98.
    let next = next_policy(&old, &[pk(&new_owner)], 1, &[pk(&new_publisher)]);
    assert_eq!(
        record(&mut journal, &f.scope, &next, &[&owner, &new_owner]).unwrap(),
        PolicyReceiptOutcome::Advanced,
    );
    assert!(matches!(
        publisher_root_signature(&mut journal, &f, &root, 1, &old_publisher),
        Err(V2PolicyReceiptError::InvalidPublisherSignature(
            crate::sync::SignedRootError::MissingQuorum))
    ));
    assert!(publisher_root_signature(&mut journal, &f, &root, 2, &new_publisher).is_ok());
    drop(journal);

    // Reopen must use the LATEST persisted publisher quorum, never cached
    // or remote-supplied old keys; stale valid signature cannot be reused.
    let mut reopened = f.open();
    assert!(matches!(
        publisher_root_signature(&mut reopened, &f, &root, 1, &old_publisher),
        Err(V2PolicyReceiptError::InvalidPublisherSignature(
            crate::sync::SignedRootError::MissingQuorum))
    ));
    assert!(publisher_root_signature(&mut reopened, &f, &root, 2, &new_publisher).is_ok());

    // Signed but non-anchored root must not be implicitly enrolled.
    let fabricated = f.root();
    assert!(matches!(
        publisher_root_signature(&mut reopened, &f, &fabricated, 2, &new_publisher),
        Err(V2PolicyReceiptError::NotCurrentlyAnchoredRoot)
    ));
}


#[test]
fn valid_enrolled_policy_blocks_legacy_unsigned_successor_even_in_e2b_only_build() {
    let f = Fixture::new();
    let owner = signer(1);
    let mut journal = f.with_pinned_root();
    let original = journal.load(&f.scope).unwrap().unwrap().root().clone();
    let (policy, bytes) = first_policy(&f, &owner);
    let policy_pin = policy.canonical_digest().unwrap();
    journal.enroll_externally_approved_genesis_policy(
        &f.scope, &policy_pin, &bytes,
    ).unwrap();
    // A legitimate, fully validated owner-policy receipt is readable.
    assert_eq!(
        journal.load_publisher_policy(&f.scope).unwrap().unwrap().digest(),
        policy_pin,
    );
    assert_eq!(journal.load(&f.scope).unwrap().unwrap().root(), &original);

    let successor = S3PackedHeadV2::new(
        &f.ns, 2, OperationId::new(),
        &blake3::hash(b"valid-policy-unsigned-gen2-manifest").to_hex().to_string(),
        OperationId::new(),
        &blake3::hash(b"valid-policy-unsigned-gen2-index").to_hex().to_string(),
        Some(original.digest(&f.ns).unwrap()),
    ).unwrap();
    let preview = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &successor, "etag-untrusted-successor",
    );
    // A complete pinned read does not grant publisher authorization.
    assert!(matches!(
        journal.record_verified_successor(
            &f.scope, &successor, "etag-untrusted-successor", &preview,
        ), Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
    drop(journal);
    let mut reopened = f.open();
    assert!(matches!(
        reopened.record_verified_successor(
            &f.scope, &successor, "etag-untrusted-successor", &preview,
        ), Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
    let anchored = reopened.load(&f.scope).unwrap().unwrap();
    assert_eq!(anchored.root(), &original);
    assert_eq!(anchored.etag(), "etag-e2b-genesis");
}

#[test]
fn signed_policy_blocks_legacy_same_root_genesis_and_successor_noop() {
    let f = Fixture::new();
    let owner = signer(1);
    let mut journal = f.with_pinned_root();
    let saved = journal.load(&f.scope).unwrap().unwrap();
    let root = saved.root().clone();
    let etag = saved.etag().to_string();
    let (policy, policy_bytes) = first_policy(&f, &owner);
    let pin = policy.canonical_digest().unwrap();
    journal.enroll_externally_approved_genesis_policy(
        &f.scope, &pin, &policy_bytes,
    ).unwrap();

    // Neither a successful read nor the fact that the exact root already
    // exists may be interpreted as signed owner/publisher authorization.
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &root, &etag,
    );
    assert!(matches!(
        journal.record_verified_successor(&f.scope, &root, &etag, &pinned),
        Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
    assert!(matches!(
        journal.enroll_verified_genesis(
            &f.scope, &root.digest(&f.ns).unwrap(), &root, &etag, &pinned,
        ),
        Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
    drop(journal);

    let mut reopened = f.open();
    assert!(matches!(
        reopened.record_verified_successor(&f.scope, &root, &etag, &pinned),
        Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root(), &root);
    assert_eq!(reopened.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), pin);
}


/// Reconstruct the ORIGINAL V2 journal schema with an already trusted root.
/// Merely opening a new E2b-format database does not test migration.
fn plant_legacy_v2_root(f: &Fixture) -> S3PackedHeadV2 {
    let db = Connection::open(f.path()).unwrap();
    db.execute_batch(
        "CREATE TABLE s3_v2_trusted_roots (
            profile_id TEXT PRIMARY KEY NOT NULL,
            vault_id TEXT NOT NULL,
            scope_fingerprint TEXT NOT NULL,
            head_key TEXT NOT NULL,
            head_json BLOB NOT NULL,
            head_etag TEXT NOT NULL,
            previous_head_json BLOB,
            receipt_digest TEXT NOT NULL,
            record_schema INTEGER NOT NULL CHECK(record_schema=1)
        );",
    ).unwrap();
    let root = f.root();
    let bytes = root.encode(&f.ns).unwrap();
    let etag = "legacy-v2-etag";
    let digest = super::super::receipt_digest(&f.scope, &bytes, etag, None);
    db.execute(
        "INSERT INTO s3_v2_trusted_roots (
            profile_id,vault_id,scope_fingerprint,head_key,head_json,
            head_etag,previous_head_json,receipt_digest,record_schema
         ) VALUES (?1,?2,?3,?4,?5,?6,NULL,?7,1)",
        params![
            f.scope.profile_id(),
            f.ns.vault_id().to_string(),
            f.scope.binding_fingerprint(),
            f.ns.current_head_key(),
            bytes, etag, digest,
        ],
    ).unwrap();
    root
}

#[test]
fn actual_legacy_root_database_migrates_in_place_without_changing_anchor() {
    let f = Fixture::new();
    let expected = plant_legacy_v2_root(&f);
    let mut upgraded = f.open();
    let anchored = upgraded.load(&f.scope).unwrap().unwrap();
    assert_eq!(anchored.root().generation(), 1);
    assert_eq!(anchored.root().digest(&f.ns).unwrap(), expected.digest(&f.ns).unwrap());
    assert_eq!(anchored.etag(), "legacy-v2-etag");
    assert!(upgraded.load_publisher_policy(&f.scope).unwrap().is_none());
    let (policy, bytes) = first_policy(&f, &signer(1));
    let pinned = policy.canonical_digest().unwrap();
    assert_eq!(upgraded.enroll_externally_approved_genesis_policy(
        &f.scope, &pinned, &bytes,
    ).unwrap(), PolicyReceiptOutcome::Inserted);
    drop(upgraded);
    let after_restart = f.open();
    assert_eq!(after_restart.load_publisher_policy(&f.scope)
        .unwrap().unwrap().digest(), pinned);
    assert_eq!(after_restart.load(&f.scope).unwrap().unwrap().etag(), "legacy-v2-etag");
    assert_eq!(after_restart.db.query_row(
        "SELECT count(*) FROM s3_v2_trusted_roots", [], |row| row.get::<_, i64>(0),
    ).unwrap(), 1);
}

#[test]
fn partially_migrated_policy_columns_refuse_database_open() {
    let f = Fixture::new();
    plant_legacy_v2_root(&f);
    let db = Connection::open(f.path()).unwrap();
    db.execute_batch(
        "ALTER TABLE s3_v2_trusted_roots ADD COLUMN policy_json BLOB;",
    ).unwrap();
    drop(db);
    let admission = RegisteredReadAdmission::open(&f.core, &f.vault).unwrap();
    assert!(matches!(
        S3V2TrustedAnchorJournal::open(&admission, &f.path()),
        Err(S3V2TrustAnchorError::CorruptedAnchor),
    ));
}

#[test]
fn incomplete_publisher_policy_row_blocks_regular_v2_root_load() {
    let f = Fixture::new();
    let mut store = f.with_pinned_root();
    let (policy, bytes) = first_policy(&f, &signer(1));
    store.enroll_externally_approved_genesis_policy(
        &f.scope, &policy.canonical_digest().unwrap(), &bytes,
    ).unwrap();
    drop(store);
    let db = Connection::open(f.path()).unwrap();
    db.execute(
        "UPDATE s3_v2_trusted_roots SET policy_receipt=NULL WHERE profile_id=?1",
        params![f.scope.profile_id()],
    ).unwrap();
    drop(db);
    let reopened = f.open();
    assert!(matches!(
        reopened.load(&f.scope),
        Err(S3V2TrustAnchorError::CorruptedAnchor),
    ));
    assert!(matches!(
        reopened.load_publisher_policy(&f.scope),
        Err(V2PolicyReceiptError::Root(S3V2TrustAnchorError::CorruptedAnchor)),
    ));
}


// E3c tests share the established E2b test's registered protected workspace,
// S3 namespace, synthetic owner signer and single guarded SQLite file.
#[cfg(feature = "signed-vault-atomic-join-lab")]
fn signed_atomic_join(
    f: &Fixture,
    journal: &mut S3V2TrustedAnchorJournal,
    root: &S3PackedHeadV2,
    policy: &PublisherPolicyV1,
    complete_read: &S3PinnedV2ReadPreview,
    observed_etag: &str,
    trusted_owner: &Ed25519KeyPair,
    signer: &Ed25519KeyPair,
    root_pin: &str,
    policy_pin: &str,
) -> Result<crate::sync::AtomicJoinOutcome, crate::sync::AtomicJoinError> {
    use crate::sync::{IndependentlyApprovedJoin, JOIN_GENESIS_TYPE};

    let owner_keys = [pk(trusted_owner)];
    let approved = IndependentlyApprovedJoin {
        bucket: &f.profile.bucket,
        keyspace: &f.ns,
        pinned_genesis_root_blake3: root_pin,
        pinned_genesis_policy_blake3: policy_pin,
        trusted_owner_keys: &owner_keys,
        owner_quorum: 1,
        recipient_challenge_hex:
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    };
    let payload = serde_json::to_vec(&json!({
        "schema": 1,
        "purpose": "review-only-join-genesis",
        "provider": "yandex-object-storage",
        "endpoint": "https://storage.yandexcloud.net",
        "region": "ru-central1",
        "force_path_style": true,
        "bucket": f.profile.bucket,
        "current_head_key": f.ns.current_head_key(),
        "vault_id": f.ns.vault_id(),
        "root_generation": root.generation(),
        "root_blake3": root_pin,
        "policy_epoch": policy.epoch(),
        "policy_blake3": policy_pin,
        "recipient_challenge_hex": approved.recipient_challenge_hex,
    })).unwrap();
    let signature = signer.sign(&super::super::super::s3_signed_root_paev1::pae(
        JOIN_GENESIS_TYPE.as_bytes(), &payload,
    ));
    let signs = [DecodedSignature {
        sig: signature.as_ref(), keyid: Some("untrusted-owner-keyid"),
    }];
    journal.enroll_signed_pinned_genesis_atomically(
        &f.scope, root, observed_etag, complete_read, policy,
        &DecodedDsseEnvelope {
            payload_type: JOIN_GENESIS_TYPE,
            payload: &payload,
            signatures: &signs,
        }, &approved,
    )
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn atomic_signed_join_persists_root_and_policy_together_after_restart() {
    use crate::sync::AtomicJoinOutcome;

    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-joined");
    let mut j = f.open();
    assert!(j.load(&f.scope).unwrap().is_none());
    assert_eq!(signed_atomic_join(&f, &mut j, &root, &policy, &pinned,
        "etag-joined", &owner, &owner, &root_pin, &policy_pin).unwrap(),
        AtomicJoinOutcome::Inserted);
    assert_eq!(j.load(&f.scope).unwrap().unwrap().root(), &root);
    assert_eq!(j.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), policy_pin);
    // Repeated same join cannot create a new genesis or mutate a baseline.
    assert_eq!(signed_atomic_join(&f, &mut j, &root, &policy, &pinned,
        "etag-joined", &owner, &owner, &root_pin, &policy_pin).unwrap(),
        AtomicJoinOutcome::Unchanged);
    drop(j);
    let reopened = f.open();
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root(), &root);
    assert_eq!(reopened.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), policy_pin);
    assert_eq!(reopened.db.query_row(
        "SELECT COUNT(*) FROM s3_v2_trusted_roots", [], |r| r.get::<_, i64>(0),
    ).unwrap(), 1);
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn invalid_owner_signature_never_creates_only_half_of_a_join() {
    use crate::sync::AtomicJoinError;

    let f = Fixture::new();
    let (trusted, attacker) = (signer(1), signer(42));
    let root = f.root();
    let (policy, _) = first_policy(&f, &trusted);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-joined");
    let mut j = f.open();
    assert!(matches!(signed_atomic_join(
        &f, &mut j, &root, &policy, &pinned, "etag-joined",
        &trusted, &attacker, &root_pin, &policy_pin,
    ), Err(AtomicJoinError::UnverifiedJoin(_))));
    assert!(j.load(&f.scope).unwrap().is_none());
    assert!(j.db.query_row(
        "SELECT COUNT(*) FROM s3_v2_trusted_roots", [], |r| r.get::<_, i64>(0),
    ).unwrap() == 0);
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn missing_complete_pinned_read_never_creates_a_trusted_policy() {
    use crate::sync::AtomicJoinError;

    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "wrong-etag");
    let mut j = f.open();
    assert!(matches!(signed_atomic_join(
        &f, &mut j, &root, &policy, &pinned, "etag-joined",
        &owner, &owner, &root_pin, &policy_pin,
    ), Err(AtomicJoinError::UnverifiedJoin(_))));
    assert!(j.load(&f.scope).unwrap().is_none());
    assert!(j.load_publisher_policy(&f.scope).is_err());
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn independently_wrong_policy_pin_cannot_create_a_partial_join() {
    use crate::sync::AtomicJoinError;

    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-joined");
    let invalid_policy_pin = "f".repeat(64);
    let mut j = f.open();
    assert!(matches!(signed_atomic_join(
        &f, &mut j, &root, &policy, &pinned, "etag-joined",
        &owner, &owner, &root_pin, &invalid_policy_pin,
    ), Err(AtomicJoinError::UnverifiedJoin(_))));
    assert!(j.load(&f.scope).unwrap().is_none());
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn previously_anchored_root_cannot_be_silently_upgraded_or_rebound() {
    use crate::sync::AtomicJoinError;

    let f = Fixture::new();
    let owner = signer(1);
    let mut j = f.with_pinned_root();
    let root = j.load(&f.scope).unwrap().unwrap().root().clone();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-e2b-genesis");
    // Legacy root-only enrollment is explicitly NOT silently converted to
    // an atomic owner-signed join by an unreviewed background process.
    assert!(matches!(signed_atomic_join(
        &f, &mut j, &root, &policy, &pinned, "etag-e2b-genesis",
        &owner, &owner, &root_pin, &policy_pin,
    ), Err(AtomicJoinError::ConflictingBinding)));
    assert!(j.load_publisher_policy(&f.scope).unwrap().is_none());
}

#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn two_handles_compete_but_only_one_atomic_genesis_is_inserted() {
    use crate::sync::{AtomicJoinError, AtomicJoinOutcome};

    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-joined");
    let mut first = f.open();
    let mut second = f.open();
    assert_eq!(signed_atomic_join(
        &f, &mut first, &root, &policy, &pinned, "etag-joined",
        &owner, &owner, &root_pin, &policy_pin,
    ).unwrap(), AtomicJoinOutcome::Inserted);
    // One other handle sees committed root+policy together, not two states.
    assert_eq!(signed_atomic_join(
        &f, &mut second, &root, &policy, &pinned, "etag-joined",
        &owner, &owner, &root_pin, &policy_pin,
    ).unwrap(), AtomicJoinOutcome::Unchanged);

    let foreign_root = f.root();
    let foreign_pin = foreign_root.digest(&f.ns).unwrap();
    let foreign_pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &foreign_root, "etag-other",
    );
    assert!(matches!(signed_atomic_join(
        &f, &mut second, &foreign_root, &policy, &foreign_pinned,
        "etag-other", &owner, &owner, &foreign_pin, &policy_pin,
    ), Err(AtomicJoinError::ConflictingBinding)));
    assert_eq!(first.load(&f.scope).unwrap().unwrap().root(), &root);
}


#[cfg(feature = "signed-vault-atomic-join-lab")]
#[test]
fn unsigned_successor_cannot_advance_signed_owner_policy_trust() {
    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &root, "etag-genesis",
    );
    let mut journal = f.open();
    signed_atomic_join(
        &f, &mut journal, &root, &policy, &pinned, "etag-genesis",
        &owner, &owner, &root_pin, &policy_pin,
    ).unwrap();

    let unsigned = S3PackedHeadV2::new(
        &f.ns, 2, OperationId::new(),
        &blake3::hash(b"unsigned-manifest").to_hex().to_string(),
        OperationId::new(),
        &blake3::hash(b"unsigned-index").to_hex().to_string(),
        Some(root_pin),
    ).unwrap();
    let full_read_without_signature = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &unsigned, "etag-unsigned",
    );
    // A fully pinned, directly linked S3 state still does not demonstrate
    // publisher authorization. Legacy promotion must HOLD for joined Vaults.
    assert!(matches!(
        journal.record_verified_successor(
            &f.scope, &unsigned, "etag-unsigned",
            &full_read_without_signature,
        ),
        Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
    assert_eq!(journal.load(&f.scope).unwrap().unwrap().root(), &root);
    drop(journal);
    let reopened = f.open();
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root(), &root);
    assert_eq!(
        reopened.load_publisher_policy(&f.scope).unwrap().unwrap().digest(),
        policy_pin,
    );
}

 
// E3d: the already-verified E3c genesis may advance only via the latest
// durably saved publisher quorum and the pre-existing complete V2 pinned read.
#[cfg(feature = "signed-vault-signed-successor-lab")]
fn successor_for(
    f: &Fixture, previous: &S3PackedHeadV2, label: &str,
) -> S3PackedHeadV2 {
    S3PackedHeadV2::new(
        &f.ns, previous.generation() + 1,
        OperationId::new(),
        &blake3::hash(format!("{label}/manifest").as_bytes()).to_hex().to_string(),
        OperationId::new(),
        &blake3::hash(format!("{label}/index").as_bytes()).to_hex().to_string(),
        Some(previous.digest(&f.ns).unwrap()),
    ).unwrap()
}

#[cfg(feature = "signed-vault-signed-successor-lab")]
fn signed_successor_with_pinned_read(
    j: &mut S3V2TrustedAnchorJournal,
    f: &Fixture,
    candidate: &S3PackedHeadV2,
    predecessor: &S3PackedHeadV2,
    publisher: &Ed25519KeyPair,
    claimed_epoch: u64,
    etag: &str,
    complete_read: &S3PinnedV2ReadPreview,
) -> Result<crate::sync::SignedSuccessorOutcome, crate::sync::SignedSuccessorError> {
    use crate::sync::ROOT_PAYLOAD_TYPE;
    let bytes = serde_json::to_vec(&json!({
        "schema": 1,
        "provider": "yandex-object-storage",
        "endpoint": "https://storage.yandexcloud.net",
        "region": "ru-central1",
        "force_path_style": true,
        "bucket": f.profile.bucket,
        "current_head_key": f.ns.current_head_key(),
        "vault_id": f.ns.vault_id(),
        "generation": candidate.generation(),
        "root_blake3": candidate.digest(&f.ns).unwrap(),
        "previous_root_blake3": predecessor.digest(&f.ns).unwrap(),
        "policy_epoch": claimed_epoch,
    })).unwrap();
    let raw = publisher.sign(&super::super::super::s3_signed_root_paev1::pae(
        ROOT_PAYLOAD_TYPE.as_bytes(), &bytes,
    ));
    let signatures = [DecodedSignature {
        sig: raw.as_ref(), keyid: Some("untrusted-keyid-hint"),
    }];
    j.record_verified_signed_successor(
        &f.scope, candidate, etag, complete_read,
        &DecodedDsseEnvelope {
            payload_type: ROOT_PAYLOAD_TYPE,
            payload: &bytes,
            signatures: &signatures,
        },
    )
}

#[cfg(feature = "signed-vault-signed-successor-lab")]
#[test]
fn signed_successor_advances_one_generation_idempotently_and_survives_restart() {
    use crate::sync::SignedSuccessorOutcome;

    let f = Fixture::new();
    let owner = signer(1);
    let publisher = signer(99);
    let genesis = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = genesis.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let mut j = f.open();
    let genesis_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &genesis, "genesis-etag",
    );
    signed_atomic_join(
        &f, &mut j, &genesis, &policy, &genesis_read, "genesis-etag",
        &owner, &owner, &root_pin, &policy_pin,
    ).unwrap();
    let next = successor_for(&f, &genesis, "valid-gen2");
    let verified = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &next, "generation2-etag",
    );
    assert_eq!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &publisher, 1,
        "generation2-etag", &verified,
    ).unwrap(), SignedSuccessorOutcome::Advanced);
    assert_eq!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &publisher, 1,
        "generation2-etag", &verified,
    ).unwrap(), SignedSuccessorOutcome::Unchanged);
    assert_eq!(j.load(&f.scope).unwrap().unwrap().root(), &next);
    assert_eq!(j.load(&f.scope).unwrap().unwrap().previous_root(), Some(&genesis));
    assert_eq!(j.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), policy_pin);
    drop(j);
    let mut reopened = f.open();
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root(), &next);
    let third = successor_for(&f, &next, "valid-gen3");
    let third_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &third, "generation3-etag",
    );
    assert_eq!(signed_successor_with_pinned_read(
        &mut reopened, &f, &third, &next, &publisher, 1,
        "generation3-etag", &third_read,
    ).unwrap(), SignedSuccessorOutcome::Advanced);
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root(), &third);
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().previous_root(), Some(&next));
    assert_eq!(reopened.load_publisher_policy(&f.scope).unwrap().unwrap().digest(), policy_pin);
}

#[cfg(feature = "signed-vault-signed-successor-lab")]
#[test]
fn unsigned_wrong_publisher_or_etag_can_never_advance_signed_root() {
    use crate::sync::{SignedRootError, SignedSuccessorError};
    let f = Fixture::new();
    let owner = signer(1);
    let genesis = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = genesis.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let mut j = f.open();
    let genesis_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &genesis, "genesis-etag",
    );
    signed_atomic_join(&f, &mut j, &genesis, &policy,
        &genesis_read, "genesis-etag", &owner, &owner,
        &root_pin, &policy_pin).unwrap();
    let next = successor_for(&f, &genesis, "candidate");
    let verified = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &next, "etag-2",
    );
    assert!(matches!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &signer(98), 1, "etag-2", &verified,
    ), Err(SignedSuccessorError::InvalidSignature(SignedRootError::MissingQuorum))));
    assert!(matches!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &signer(99), 1, "wrong-etag", &verified,
    ), Err(SignedSuccessorError::InvalidPinnedRead)));
    assert!(matches!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &signer(99), 2, "etag-2", &verified,
    ), Err(SignedSuccessorError::InvalidSignature(SignedRootError::WrongScope))));
    assert_eq!(j.load(&f.scope).unwrap().unwrap().root(), &genesis);
}

#[cfg(feature = "signed-vault-signed-successor-lab")]
#[test]
fn revoked_publisher_signature_is_rechecked_inside_root_transaction() {
    use crate::sync::{SignedRootError, SignedSuccessorError, SignedSuccessorOutcome};
    let f = Fixture::new();
    let owner = signer(1);
    let new_owner = signer(2);
    let old_publisher = signer(99);
    let new_publisher = signer(98);
    let genesis = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = genesis.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let mut j = f.open();
    let genesis_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &genesis, "genesis-etag",
    );
    signed_atomic_join(&f, &mut j, &genesis, &policy,
        &genesis_read, "genesis-etag", &owner, &owner,
        &root_pin, &policy_pin).unwrap();
    let next = successor_for(&f, &genesis, "revocation-gen2");
    let verified = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &next, "etag-2",
    );
    let mut other = f.open();
    let rotated_bytes = next_policy(
        &policy, &[pk(&new_owner)], 1, &[pk(&new_publisher)],
    );
    assert_eq!(record(
        &mut other, &f.scope, &rotated_bytes, &[&owner, &new_owner],
    ).unwrap(), PolicyReceiptOutcome::Advanced);
    assert!(matches!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &old_publisher, 1, "etag-2", &verified,
    ), Err(SignedSuccessorError::InvalidSignature(SignedRootError::MissingQuorum))));
    assert_eq!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &new_publisher, 2, "etag-2", &verified,
    ).unwrap(), SignedSuccessorOutcome::Advanced);
    assert_eq!(other.load(&f.scope).unwrap().unwrap().root(), &next);
    assert_eq!(other.load_publisher_policy(&f.scope).unwrap().unwrap().policy().epoch(), 2);
}

#[cfg(feature = "signed-vault-signed-successor-lab")]
#[test]
fn two_handles_reject_signed_sibling_fork_and_legacy_root_promotion() {
    use crate::sync::{SignedSuccessorError, SignedSuccessorOutcome};
    let f = Fixture::new();
    let owner = signer(1);
    let publisher = signer(99);
    let genesis = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = genesis.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let mut first = f.open();
    let genesis_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &genesis, "genesis-etag",
    );
    signed_atomic_join(&f, &mut first, &genesis, &policy,
        &genesis_read, "genesis-etag", &owner, &owner,
        &root_pin, &policy_pin).unwrap();
    let mut second = f.open();
    let accepted = successor_for(&f, &genesis, "accepted-gen2");
    let sibling = successor_for(&f, &genesis, "sibling-fork-gen2");
    let accepted_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &accepted, "etag-accepted",
    );
    let sibling_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &sibling, "etag-fork",
    );
    assert_eq!(signed_successor_with_pinned_read(
        &mut first, &f, &accepted, &genesis, &publisher, 1,
        "etag-accepted", &accepted_read,
    ).unwrap(), SignedSuccessorOutcome::Advanced);
    assert!(matches!(signed_successor_with_pinned_read(
        &mut second, &f, &sibling, &genesis, &publisher, 1,
        "etag-fork", &sibling_read,
    ), Err(SignedSuccessorError::Root(S3V2TrustAnchorError::RollbackOrFork))));
    assert!(matches!(second.record_verified_successor(
        &f.scope, &sibling, "etag-fork", &sibling_read,
    ), Err(S3V2TrustAnchorError::UnverifiedRead)));
    assert_eq!(second.load(&f.scope).unwrap().unwrap().root(), &accepted);
}

#[cfg(feature = "signed-vault-signed-successor-lab")]
#[test]
fn root_only_policy_missing_cannot_bootstrap_signed_successor() {
    use crate::sync::SignedSuccessorError;
    let f = Fixture::new();
    let mut j = f.with_pinned_root();
    let genesis = j.load(&f.scope).unwrap().unwrap().root().clone();
    let next = successor_for(&f, &genesis, "no-policy-gen2");
    let verified = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &next, "etag-2",
    );
    assert!(matches!(signed_successor_with_pinned_read(
        &mut j, &f, &next, &genesis, &signer(99), 1, "etag-2", &verified,
    ), Err(SignedSuccessorError::Policy(V2PolicyReceiptError::MissingPolicy))));
    assert_eq!(j.load(&f.scope).unwrap().unwrap().root(), &genesis);
}


#[cfg(feature = "signed-vault-signed-successor-lab")]
#[test]
fn two_distinct_publisher_signatures_required_for_successor_quorum() {
    use crate::sync::{SignedRootError, SignedSuccessorError, SignedSuccessorOutcome,
        ROOT_PAYLOAD_TYPE};
    let f = Fixture::new();
    let owner = signer(1);
    let first_publisher = signer(99);
    let second_publisher = signer(98);
    let genesis = f.root();
    let (initial_policy, _) = first_policy(&f, &owner);
    let mut candidate_policy = serde_json::to_value(&initial_policy).unwrap();
    candidate_policy["publisher_keys"] = json!([
        pk(&first_publisher), pk(&second_publisher),
    ]);
    candidate_policy["publisher_quorum"] = json!(2);
    let policy = PublisherPolicyV1::parse_for_review(
        &serde_json::to_vec(&candidate_policy).unwrap(),
    ).unwrap();
    let root_pin = genesis.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let mut journal = f.open();
    let genesis_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &genesis, "etag-genesis",
    );
    signed_atomic_join(
        &f, &mut journal, &genesis, &policy, &genesis_read,
        "etag-genesis", &owner, &owner, &root_pin, &policy_pin,
    ).unwrap();

    let successor = successor_for(&f, &genesis, "two-publisher-quorum");
    let etag = "etag-multi";
    let preview = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &successor, etag,
    );
    let payload = serde_json::to_vec(&json!({
        "schema": 1,
        "provider": "yandex-object-storage",
        "endpoint": "https://storage.yandexcloud.net",
        "region": "ru-central1",
        "force_path_style": true,
        "bucket": f.profile.bucket,
        "current_head_key": f.ns.current_head_key(),
        "vault_id": f.ns.vault_id(),
        "generation": successor.generation(),
        "root_blake3": successor.digest(&f.ns).unwrap(),
        "previous_root_blake3": root_pin,
        "policy_epoch": 1,
    })).unwrap();
    let pae = super::super::super::s3_signed_root_paev1::pae(
        ROOT_PAYLOAD_TYPE.as_bytes(), &payload,
    );
    let sig_a = first_publisher.sign(&pae).as_ref().to_vec();
    let sig_b = second_publisher.sign(&pae).as_ref().to_vec();

    let submit = |journal: &mut S3V2TrustedAnchorJournal, sigs: &[DecodedSignature<'_>]| {
        journal.record_verified_signed_successor(
            &f.scope, &successor, etag, &preview,
            &DecodedDsseEnvelope {
                payload_type: ROOT_PAYLOAD_TYPE,
                payload: &payload,
                signatures: sigs,
            },
        )
    };
    // Repeating the same cryptographic key under different keyid labels
    // must NEVER simulate a two-publisher threshold.
    let duplicated = [
        DecodedSignature { sig: &sig_a, keyid: Some("owner-one") },
        DecodedSignature { sig: &sig_a, keyid: Some("owner-two-forged") },
    ];
    assert!(matches!(
        submit(&mut journal, &duplicated),
        Err(SignedSuccessorError::InvalidSignature(SignedRootError::MissingQuorum))
    ));
    assert_eq!(journal.load(&f.scope).unwrap().unwrap().root(), &genesis);

    let approved = [
        DecodedSignature { sig: &sig_a, keyid: Some("arbitrary-a") },
        DecodedSignature { sig: &sig_b, keyid: Some("arbitrary-b") },
    ];
    assert_eq!(submit(&mut journal, &approved).unwrap(), SignedSuccessorOutcome::Advanced);
    assert_eq!(journal.load(&f.scope).unwrap().unwrap().root(), &successor);
}


// E3e: a receiver-issued public challenge is single-use within one local
// process session. It is never imported from untrusted S3 or stored as a new
// source of Vault trust. Human approval of keys/pins remains a prerequisite.
#[cfg(feature = "signed-vault-receiver-session-lab")]
fn receiver_session_join(
    f: &Fixture,
    session: &mut crate::sync::ReceiverJoinSession,
    journal: &mut S3V2TrustedAnchorJournal,
    root: &S3PackedHeadV2,
    policy: &PublisherPolicyV1,
    pinned: &S3PinnedV2ReadPreview,
    trusted_owner: &Ed25519KeyPair,
    signing_key: &Ed25519KeyPair,
    approved_challenge_override: Option<&str>,
    scope_override: Option<&S3RemoteAnchorScope>,
) -> Result<crate::sync::AtomicJoinOutcome, crate::sync::ReceiverJoinSessionError> {
    use crate::sync::{IndependentlyApprovedJoin, JOIN_GENESIS_TYPE};

    let challenge = session.challenge_hex();
    let owner_keys = [pk(trusted_owner)];
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let approved = IndependentlyApprovedJoin {
        bucket: &f.profile.bucket,
        keyspace: &f.ns,
        pinned_genesis_root_blake3: &root_pin,
        pinned_genesis_policy_blake3: &policy_pin,
        trusted_owner_keys: &owner_keys,
        owner_quorum: 1,
        recipient_challenge_hex: approved_challenge_override.unwrap_or(&challenge),
    };
    let payload = serde_json::to_vec(&json!({
        "schema": 1,
        "purpose": "review-only-join-genesis",
        "provider": "yandex-object-storage",
        "endpoint": "https://storage.yandexcloud.net",
        "region": "ru-central1",
        "force_path_style": true,
        "bucket": f.profile.bucket,
        "current_head_key": f.ns.current_head_key(),
        "vault_id": f.ns.vault_id(),
        "root_generation": root.generation(),
        "root_blake3": root_pin.as_str(),
        "policy_epoch": policy.epoch(),
        "policy_blake3": policy_pin.as_str(),
        "recipient_challenge_hex": challenge.as_str(),
    })).unwrap();
    let signature = signing_key.sign(
        &super::super::super::s3_signed_root_paev1::pae(
            JOIN_GENESIS_TYPE.as_bytes(), &payload,
        ),
    );
    let signatures = [DecodedSignature {
        sig: signature.as_ref(), keyid: Some("untrusted-label"),
    }];
    session.enroll_once(
        journal, scope_override.unwrap_or(&f.scope),
        root, "etag-e3e", pinned, policy,
        &DecodedDsseEnvelope {
            payload_type: JOIN_GENESIS_TYPE,
            payload: &payload,
            signatures: &signatures,
        },
        &approved,
    )
}

#[cfg(feature = "signed-vault-receiver-session-lab")]
#[test]
fn receiver_session_csprng_nonce_and_one_shot_atomic_join() {
    use crate::sync::{AtomicJoinOutcome, ReceiverJoinSession, ReceiverJoinSessionError};
    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let mut session = ReceiverJoinSession::issue(&f.scope).unwrap();
    let other = ReceiverJoinSession::issue(&f.scope).unwrap();
    let hex = session.challenge_hex();
    assert_eq!(hex.len(), 64);
    assert!(hex.as_bytes().iter().all(|b|
        b.is_ascii_digit() || (b'a'..=b'f').contains(b)));
    assert!(hex.as_bytes().iter().any(|b| *b != b'0'));
    // CSPRNG-generated independent sessions must not reuse a challenge.
    assert_ne!(hex, other.challenge_hex());
    let mut journal = f.open();
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-e3e");
    assert_eq!(receiver_session_join(
        &f, &mut session, &mut journal, &root, &policy, &pinned,
        &owner, &owner, None, None,
    ).unwrap(), AtomicJoinOutcome::Inserted);
    assert_eq!(journal.load(&f.scope).unwrap().unwrap().root(), &root);
    assert!(matches!(receiver_session_join(
        &f, &mut session, &mut journal, &root, &policy, &pinned,
        &owner, &owner, None, None,
    ), Err(ReceiverJoinSessionError::AlreadyConsumed)));
}

#[cfg(feature = "signed-vault-receiver-session-lab")]
#[test]
fn wrong_owner_signature_spends_session_without_any_partial_root() {
    use crate::sync::{ReceiverJoinSession, ReceiverJoinSessionError};
    let f = Fixture::new();
    let owner = signer(1);
    let attacker = signer(42);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-e3e");
    let mut journal = f.open();
    let mut session = ReceiverJoinSession::issue(&f.scope).unwrap();
    assert!(matches!(receiver_session_join(
        &f, &mut session, &mut journal, &root, &policy, &pinned,
        &owner, &attacker, None, None,
    ), Err(ReceiverJoinSessionError::Join(_))));
    assert!(matches!(receiver_session_join(
        &f, &mut session, &mut journal, &root, &policy, &pinned,
        &owner, &owner, None, None,
    ), Err(ReceiverJoinSessionError::AlreadyConsumed)));
    assert!(journal.load(&f.scope).unwrap().is_none());
}

#[cfg(feature = "signed-vault-receiver-session-lab")]
#[test]
fn expired_foreign_scope_or_changed_challenge_can_never_enroll() {
    use crate::sync::{ReceiverJoinSession, ReceiverJoinSessionError};
    let f = Fixture::new();
    let owner = signer(1);
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let pinned = S3PinnedV2ReadPreview::for_trust_anchor_test(&f.scope, &root, "etag-e3e");
    let mut journal = f.open();

    let mut expired = ReceiverJoinSession::issue(&f.scope).unwrap();
    expired.expire_for_test();
    assert!(matches!(receiver_session_join(
        &f, &mut expired, &mut journal, &root, &policy, &pinned,
        &owner, &owner, None, None,
    ), Err(ReceiverJoinSessionError::Expired)));

    let mut wrong_challenge = ReceiverJoinSession::issue(&f.scope).unwrap();
    let altered = "a".repeat(64);
    assert_ne!(wrong_challenge.challenge_hex(), altered);
    assert!(matches!(receiver_session_join(
        &f, &mut wrong_challenge, &mut journal, &root, &policy, &pinned,
        &owner, &owner, Some(&altered), None,
    ), Err(ReceiverJoinSessionError::WrongChallenge)));

    let foreign_profile = S3ConnectionProfile {
        bucket: "other-e3e-bucket".into(), ..f.profile.clone()
    };
    let foreign = S3RemoteAnchorScope::new(
        f.scope.profile_id(), "independently-approved-account-binding",
        &foreign_profile, &f.ns,
    ).unwrap();
    let mut wrong_scope = ReceiverJoinSession::issue(&f.scope).unwrap();
    assert!(matches!(receiver_session_join(
        &f, &mut wrong_scope, &mut journal, &root, &policy, &pinned,
        &owner, &owner, None, Some(&foreign),
    ), Err(ReceiverJoinSessionError::WrongLocalScope)));
    assert!(journal.load(&f.scope).unwrap().is_none());
}



#[cfg(feature = "signed-vault-join-successor-integration-lab")]
#[test]
fn one_shot_receiver_join_then_signed_direct_successor_preserves_same_sqlite_policy() {
    use crate::sync::{AtomicJoinOutcome, ReceiverJoinSession,
        ReceiverJoinSessionError, SignedSuccessorError, SignedSuccessorOutcome,
        SignedRootError};
    let f = Fixture::new();
    let owner = signer(1);
    let publisher = signer(99);
    let unauthorized = signer(98);
    let genesis = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let policy_pin = policy.canonical_digest().unwrap();
    let mut session = ReceiverJoinSession::issue(&f.scope).unwrap();
    let mut journal = f.open();
    // Real owner-key approval, live Yandex and the full V2 network read
    // remain independent prerequisites. This test exercises shared SQLite
    // state using a synthetic pinned receipt and a signed invitation.
    // receiver_session_join hardcodes its test-only genesis ETag "etag-e3e".
    let matched_genesis_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &genesis, "etag-e3e",
    );
    assert_eq!(
        receiver_session_join(
            &f, &mut session, &mut journal, &genesis, &policy,
            &matched_genesis_read, &owner, &owner, None, None,
        ).unwrap(),
        AtomicJoinOutcome::Inserted,
    );
    assert!(matches!(
        receiver_session_join(
            &f, &mut session, &mut journal, &genesis, &policy,
            &matched_genesis_read, &owner, &owner, None, None,
        ),
        Err(ReceiverJoinSessionError::AlreadyConsumed),
    ));
    let second = successor_for(&f, &genesis, "e3g-successor");
    let second_read = S3PinnedV2ReadPreview::for_trust_anchor_test(
        &f.scope, &second, "etag-e3g-second",
    );
    assert!(matches!(
        signed_successor_with_pinned_read(
            &mut journal, &f, &second, &genesis, &unauthorized,
            1, "etag-e3g-second", &second_read,
        ),
        Err(SignedSuccessorError::InvalidSignature(SignedRootError::MissingQuorum)),
    ));
    assert_eq!(journal.load(&f.scope).unwrap().unwrap().root(), &genesis);
    assert_eq!(
        signed_successor_with_pinned_read(
            &mut journal, &f, &second, &genesis, &publisher,
            1, "etag-e3g-second", &second_read,
        ).unwrap(),
        SignedSuccessorOutcome::Advanced,
    );
    drop(journal);
    let mut reopened = f.open();
    assert_eq!(reopened.load(&f.scope).unwrap().unwrap().root(), &second);
    assert_eq!(
        reopened.load(&f.scope).unwrap().unwrap().previous_root(),
        Some(&genesis),
    );
    assert_eq!(
        reopened.load_publisher_policy(&f.scope).unwrap().unwrap().digest(),
        policy_pin,
    );
    assert!(matches!(
        reopened.record_verified_successor(
            &f.scope, &second, "etag-e3g-second", &second_read,
        ),
        Err(S3V2TrustAnchorError::UnverifiedRead),
    ));
}


/// E3g's real Yandex read-only handoff is available only after the receiver
/// spends its one-shot session. Invalid owner evidence must fail locally
/// BEFORE any network access or SQLite root/policy enrollment. This test does
/// not use credentials, a cloud account, or any production user objects.
#[cfg(feature = "signed-vault-join-successor-integration-lab")]
#[tokio::test]
async fn receiver_live_yandex_path_denies_invalid_owner_before_network() {
    use crate::sync::{
        AwsS3ProbeClient, IndependentlyApprovedJoin, PinnedJoinError,
        ReceiverJoinSession, ReceiverJoinSessionError, JOIN_GENESIS_TYPE,
    };
    let f = Fixture::new();
    let owner = signer(1);
    let owner_keys = [pk(&owner)];
    let root = f.root();
    let (policy, _) = first_policy(&f, &owner);
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    // Construct the SDK client, but never issue a request: the remote
    // endpoint is the genuine Yandex URL and no credentials are supplied.
    let client = AwsS3ProbeClient::connect(f.profile.clone(), f.ns.clone())
        .await.unwrap();
    let mut journal = f.open();
    let mut session = ReceiverJoinSession::issue(&f.scope).unwrap();
    let challenge = session.challenge_hex();
    let approved = IndependentlyApprovedJoin {
        bucket: &f.profile.bucket,
        keyspace: &f.ns,
        pinned_genesis_root_blake3: &root_pin,
        pinned_genesis_policy_blake3: &policy_pin,
        trusted_owner_keys: &owner_keys,
        owner_quorum: 1,
        recipient_challenge_hex: &challenge,
    };
    let forged = DecodedDsseEnvelope {
        payload_type: JOIN_GENESIS_TYPE,
        payload: b"not-an-approved-owner-invitation",
        signatures: &[],
    };
    assert!(matches!(
        session.enroll_from_yandex_read_once(
            &client, &mut journal, &f.scope,
            &root, &policy, &forged, &approved,
        ).await,
        Err(ReceiverJoinSessionError::LiveYandexRead(PinnedJoinError::Join(_)))
    ));
    assert!(matches!(
        session.enroll_from_yandex_read_once(
            &client, &mut journal, &f.scope,
            &root, &policy, &forged, &approved,
        ).await,
        Err(ReceiverJoinSessionError::AlreadyConsumed),
    ));
    assert!(journal.load(&f.scope).unwrap().is_none());
    assert!(journal.load_publisher_policy(&f.scope).is_err());
}


/// An independently entered, full fingerprint mismatch burns a fresh
/// receiver challenge before *any* signed invitation or S3 operation.
#[cfg(feature = "signed-vault-manual-owner-review-lab")]
#[tokio::test]
async fn independent_fingerprint_mismatch_burns_session_before_yandex_get() {
    use crate::sync::{
        AwsS3ProbeClient, IndependentlyApprovedJoin,
        IndependentOwnerFingerprintError, ReceiverJoinSession,
        ReceiverJoinSessionError, JOIN_GENESIS_TYPE,
        owner_join_fingerprint_hex,
    };
    let f = Fixture::new();
    let root = f.root();
    let owner = signer(1);
    let (policy, _) = first_policy(&f, &owner);
    let keys = [pk(&owner)];
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let client = AwsS3ProbeClient::connect(f.profile.clone(), f.ns.clone())
        .await.unwrap();
    let mut journal = f.open();
    let mut session = ReceiverJoinSession::issue(&f.scope).unwrap();
    let challenge = session.challenge_hex();
    let approved = IndependentlyApprovedJoin {
        bucket: &f.profile.bucket,
        keyspace: &f.ns,
        pinned_genesis_root_blake3: &root_pin,
        pinned_genesis_policy_blake3: &policy_pin,
        trusted_owner_keys: &keys,
        owner_quorum: 1,
        recipient_challenge_hex: &challenge,
    };
    let verified_fingerprint = owner_join_fingerprint_hex(&approved).unwrap();
    let another_fingerprint = if verified_fingerprint == "f".repeat(64) {
        "e".repeat(64)
    } else {
        "f".repeat(64)
    };
    let bogus_offer = DecodedDsseEnvelope {
        payload_type: JOIN_GENESIS_TYPE,
        payload: b"not-the-owner-signed-invitation",
        signatures: &[],
    };
    assert!(matches!(
        session.enroll_after_independent_owner_review_once(
            &client, &mut journal, &f.scope, &root, &policy,
            &bogus_offer, &approved, &another_fingerprint,
        ).await,
        Err(ReceiverJoinSessionError::ManualOwnerReview(
            IndependentOwnerFingerprintError::Mismatch,
        ))
    ));
    assert!(matches!(
        session.enroll_after_independent_owner_review_once(
            &client, &mut journal, &f.scope, &root, &policy,
            &bogus_offer, &approved, &verified_fingerprint,
        ).await,
        Err(ReceiverJoinSessionError::AlreadyConsumed)
    ));
    assert!(journal.load(&f.scope).unwrap().is_none());
}

/// Even a perfectly matched independent fingerprint does NOT replace the
/// DSSE owner signature, the bounded V2 read or the SQLite admission gate.
#[cfg(feature = "signed-vault-manual-owner-review-lab")]
#[tokio::test]
async fn matched_independent_fingerprint_does_not_bypass_owner_signature() {
    use crate::sync::{
        AwsS3ProbeClient, IndependentlyApprovedJoin, PinnedJoinError,
        ReceiverJoinSession, ReceiverJoinSessionError, JOIN_GENESIS_TYPE,
        owner_join_fingerprint_hex,
    };
    let f = Fixture::new();
    let root = f.root();
    let owner = signer(1);
    let (policy, _) = first_policy(&f, &owner);
    let keys = [pk(&owner)];
    let root_pin = root.digest(&f.ns).unwrap();
    let policy_pin = policy.canonical_digest().unwrap();
    let client = AwsS3ProbeClient::connect(f.profile.clone(), f.ns.clone())
        .await.unwrap();
    let mut journal = f.open();
    let mut session = ReceiverJoinSession::issue(&f.scope).unwrap();
    let challenge = session.challenge_hex();
    let approved = IndependentlyApprovedJoin {
        bucket: &f.profile.bucket,
        keyspace: &f.ns,
        pinned_genesis_root_blake3: &root_pin,
        pinned_genesis_policy_blake3: &policy_pin,
        trusted_owner_keys: &keys,
        owner_quorum: 1,
        recipient_challenge_hex: &challenge,
    };
    let independently_entered = owner_join_fingerprint_hex(&approved).unwrap();
    let forged = DecodedDsseEnvelope {
        payload_type: JOIN_GENESIS_TYPE,
        payload: b"forged-owner-invitation",
        signatures: &[],
    };
    assert!(matches!(
        session.enroll_after_independent_owner_review_once(
            &client, &mut journal, &f.scope, &root, &policy,
            &forged, &approved, &independently_entered,
        ).await,
        Err(ReceiverJoinSessionError::LiveYandexRead(PinnedJoinError::Join(_)))
    ));
    assert!(journal.load(&f.scope).unwrap().is_none());
    assert!(matches!(
        session.enroll_after_independent_owner_review_once(
            &client, &mut journal, &f.scope, &root, &policy,
            &forged, &approved, &independently_entered,
        ).await,
        Err(ReceiverJoinSessionError::AlreadyConsumed)
    ));
}
