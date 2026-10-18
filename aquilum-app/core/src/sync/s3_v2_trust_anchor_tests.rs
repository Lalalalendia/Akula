use super::*;
use crate::app_core::{Core, CoreEvent};
use crate::sync::{
    OperationId, S3ConnectionProfile, S3Keyspace, VaultId,
};
use std::path::PathBuf;
use std::sync::Arc;

struct Fixture {
    tmp: tempfile::TempDir,
    core: Arc<Core>,
    root: PathBuf,
    appdata: PathBuf,
    ns: S3Keyspace,
    profile: S3ConnectionProfile,
    scope: S3RemoteAnchorScope,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("user-vault");
        let appdata = tmp.path().join("private-app-data");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&appdata).unwrap();
        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        let ns = S3Keyspace::new("akula/shared", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "private-test-bucket".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        };
        let scope = S3RemoteAnchorScope::new(
            "desktop-v2", "account-binding-from-verified-provider", &profile, &ns,
        ).unwrap();
        Self { tmp, core, root, appdata, ns, profile, scope }
    }

    fn genesis(&self) -> S3PackedHeadV2 {
        S3PackedHeadV2::new(
            &self.ns, 1, OperationId::new(),
            &blake3::hash(b"v2-genesis-manifest").to_hex().to_string(),
            OperationId::new(),
            &blake3::hash(b"v2-genesis-index").to_hex().to_string(),
            None,
        ).unwrap()
    }

    fn next(&self, old: &S3PackedHeadV2) -> S3PackedHeadV2 {
        S3PackedHeadV2::new(
            &self.ns, old.generation() + 1,
            OperationId::new(),
            &blake3::hash(format!("manifest-g{}", old.generation()+1).as_bytes())
                .to_hex().to_string(),
            OperationId::new(),
            &blake3::hash(format!("index-g{}", old.generation()+1).as_bytes())
                .to_hex().to_string(),
            Some(old.digest(&self.ns).unwrap()),
        ).unwrap()
    }

    fn journal_path(&self) -> PathBuf {
        self.appdata.join("v2-trusted.sqlite")
    }
}

fn digest(root: &S3PackedHeadV2, scope: &S3RemoteAnchorScope) -> String {
    root.digest(scope.keyspace()).unwrap()
}

fn verified(
    root: &S3PackedHeadV2,
    etag: &str,
    scope: &S3RemoteAnchorScope,
) -> S3PinnedV2ReadPreview {
    S3PinnedV2ReadPreview::for_trust_anchor_test(scope, root, etag)
}

#[test]
fn explicit_independent_genesis_pin_survives_restart_and_cannot_bootstrap_from_cloud_alone() {
    let f = Fixture::new();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let mut journal = S3V2TrustedAnchorJournal::open(
        &admission, &f.journal_path(),
    ).unwrap();
    assert!(journal.load(&f.scope).unwrap().is_none());
    let root = f.genesis();
    let pin = digest(&root, &f.scope);
    let receipt = verified(&root, "\"etag-1\"", &f.scope);
    assert!(matches!(
        journal.enroll_verified_genesis(
            &f.scope, &"0".repeat(64), &root, "\"etag-1\"", &receipt
        ), Err(S3V2TrustAnchorError::InvalidExternalPin)
    ));
    assert!(matches!(
        journal.enroll_verified_genesis(
            &f.scope, &pin, &root, "\"etag-2\"", &receipt
        ), Err(S3V2TrustAnchorError::UnverifiedRead)
    ));
    assert!(journal.load(&f.scope).unwrap().is_none());
    assert_eq!(journal.enroll_verified_genesis(
        &f.scope, &pin, &root, "\"etag-1\"", &receipt,
    ).unwrap(), S3V2TrustAnchorOutcome::Inserted);
    assert_eq!(journal.enroll_verified_genesis(
        &f.scope, &pin, &root, "\"etag-1\"", &receipt,
    ).unwrap(), S3V2TrustAnchorOutcome::Unchanged);
    drop(journal);
    let reopened = S3V2TrustedAnchorJournal::open(&admission, &f.journal_path()).unwrap();
    let stored = reopened.load(&f.scope).unwrap().unwrap();
    assert_eq!(stored.root(), &root);
    assert_eq!(stored.etag(), "\"etag-1\"");
    assert_eq!(stored.previous_root(), None);
}

#[test]
fn durable_two_successors_preserve_exact_previous_trusted_root() {
    let f = Fixture::new();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let mut store = S3V2TrustedAnchorJournal::open(&admission, &f.journal_path()).unwrap();
    let first = f.genesis();
    store.enroll_verified_genesis(
        &f.scope, &digest(&first,&f.scope), &first,
        "etag-genesis", &verified(&first,"etag-genesis",&f.scope),
    ).unwrap();
    let second = f.next(&first);
    assert_eq!(store.record_verified_successor(
        &f.scope, &second, "etag-second", &verified(&second,"etag-second",&f.scope),
    ).unwrap(), S3V2TrustAnchorOutcome::Advanced);
    assert_eq!(store.record_verified_successor(
        &f.scope, &second, "etag-second", &verified(&second,"etag-second",&f.scope),
    ).unwrap(), S3V2TrustAnchorOutcome::Unchanged);
    assert!(matches!(
        store.record_verified_successor(
            &f.scope, &second, "etag-rewrite",
            &verified(&second,"etag-rewrite",&f.scope),
        ), Err(S3V2TrustAnchorError::RootRewritten)
    ));
    let third = f.next(&second);
    assert_eq!(store.record_verified_successor(
        &f.scope, &third, "etag-third", &verified(&third,"etag-third",&f.scope),
    ).unwrap(), S3V2TrustAnchorOutcome::Advanced);
    drop(store);
    let reopened = S3V2TrustedAnchorJournal::open(&admission, &f.journal_path()).unwrap();
    let observed = reopened.load(&f.scope).unwrap().unwrap();
    assert_eq!(observed.root(), &third);
    assert_eq!(observed.previous_root(), Some(&second));
    assert_eq!(observed.etag(), "etag-third");
}

#[test]
fn skipped_generation_rollback_fork_and_missing_prior_never_advance_sqlite() {
    let f = Fixture::new();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let mut store = S3V2TrustedAnchorJournal::open(&admission, &f.journal_path()).unwrap();
    let root = f.genesis();
    let successor = f.next(&root);
    assert!(matches!(
        store.record_verified_successor(
            &f.scope, &successor, "etag-2",
            &verified(&successor,"etag-2",&f.scope),
        ), Err(S3V2TrustAnchorError::MissingTrustedAnchor)
    ));
    store.enroll_verified_genesis(
        &f.scope, &digest(&root,&f.scope), &root,
        "etag-1", &verified(&root,"etag-1",&f.scope),
    ).unwrap();
    let skipped = f.next(&successor);
    assert!(matches!(
        store.record_verified_successor(
            &f.scope, &skipped, "etag-3",
            &verified(&skipped,"etag-3",&f.scope),
        ), Err(S3V2TrustAnchorError::IncompleteAncestry)
    ));
    let sibling_fork = f.next(&root);
    store.record_verified_successor(
        &f.scope, &successor, "etag-2",
        &verified(&successor,"etag-2",&f.scope),
    ).unwrap();
    assert!(matches!(
        store.record_verified_successor(
            &f.scope, &root, "etag-rollback",
            &verified(&root,"etag-rollback",&f.scope),
        ), Err(S3V2TrustAnchorError::RollbackOrFork)
    ));
    assert!(matches!(
        store.record_verified_successor(
            &f.scope, &sibling_fork, "etag-fork",
            &verified(&sibling_fork,"etag-fork",&f.scope),
        ), Err(S3V2TrustAnchorError::RollbackOrFork)
    ));
    assert_eq!(store.load(&f.scope).unwrap().unwrap().root(), &successor);
}

#[test]
fn profile_account_and_vault_rebinding_are_blocked_without_overwrite() {
    let f = Fixture::new();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let mut store = S3V2TrustedAnchorJournal::open(&admission, &f.journal_path()).unwrap();
    let root = f.genesis();
    store.enroll_verified_genesis(
        &f.scope, &digest(&root,&f.scope), &root,
        "etag-a", &verified(&root,"etag-a",&f.scope),
    ).unwrap();
    let other_account = S3RemoteAnchorScope::new(
        "desktop-v2", "another-account", &f.profile, &f.ns,
    ).unwrap();
    assert!(matches!(
        store.load(&other_account), Err(S3V2TrustAnchorError::ScopeChanged)
    ));
    let other_profile = S3ConnectionProfile {
        bucket: "surprise-bucket".into(), ..f.profile.clone()
    };
    let different_bucket = S3RemoteAnchorScope::new(
        "desktop-v2", "account-binding-from-verified-provider",
        &other_profile, &f.ns,
    ).unwrap();
    assert!(matches!(
        store.load(&different_bucket), Err(S3V2TrustAnchorError::ScopeChanged)
    ));
    let foreign = S3Keyspace::new("akula/shared", VaultId::new()).unwrap();
    let different_vault = S3RemoteAnchorScope::new(
        "desktop-v2", "account-binding-from-verified-provider",
        &f.profile, &foreign,
    ).unwrap();
    assert!(matches!(
        store.load(&different_vault), Err(S3V2TrustAnchorError::ScopeChanged)
    ));
    assert_eq!(store.load(&f.scope).unwrap().unwrap().root(), &root);
}

#[test]
fn corrupted_bytes_and_fake_receipt_are_not_authoritative() {
    let f = Fixture::new();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let mut store = S3V2TrustedAnchorJournal::open(&admission, &f.journal_path()).unwrap();
    let root = f.genesis();
    store.enroll_verified_genesis(
        &f.scope, &digest(&root,&f.scope), &root,
        "etag-1", &verified(&root,"etag-1",&f.scope),
    ).unwrap();
    let injected = rusqlite::Connection::open(f.journal_path()).unwrap();
    injected.execute(
        "UPDATE s3_v2_trusted_roots SET head_json=?1 WHERE profile_id=?2",
        params![b"{\"schema\":3}".as_slice(), f.scope.profile_id()],
    ).unwrap();
    assert!(matches!(
        store.load(&f.scope), Err(S3V2TrustAnchorError::CorruptedAnchor)
    ));
    injected.execute(
        "UPDATE s3_v2_trusted_roots SET record_schema=44 WHERE profile_id=?1",
        params![f.scope.profile_id()],
    ).unwrap_err(); // SQLite CHECK constraint rejects unknown on disk.
}

#[test]
fn protected_appdata_journal_refuses_user_vault_location() {
    let f = Fixture::new();
    let admission = RegisteredReadAdmission::open(&f.core, &f.root).unwrap();
    let wrong = f.root.join("user-note-folder.sqlite");
    assert!(matches!(
        S3V2TrustedAnchorJournal::open(&admission, &wrong),
        Err(S3V2TrustAnchorError::InvalidStoreLocation)
    ));
    let allowed = f.journal_path();
    assert!(S3V2TrustedAnchorJournal::open(&admission, &allowed).is_ok());
    // Parent directory remains an app-data host responsibility; the
    // root-admission guard prevents placing journal/WAL inside Markdown.
    assert!(f.tmp.path().exists());
}
