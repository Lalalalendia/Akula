//! Full disposable MinIO transaction in two independent Core registrations.
//! No user vault or external endpoint is used. The production receiver's
//! existing kernel File Gate and profile-binding paths are both exercised.
//! This test does NOT replace installed-Windows/two-physical-PC acceptance.

use super::*;
use crate::app_core::CoreEvent;
use crate::sync::filegate_custody_identity::SqliteCustodyIdentityStore;
use crate::sync::filegate_lock_custody::scoped_leaf;
use crate::sync::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
use crate::sync::profile_binding::SqliteRegisteredS3Bindings;
use crate::sync::s3_pair_invite::{open_s3_pair_invite, seal_s3_pair_invite};
use crate::sync::{S3ConnectionProfile, S3Keyspace};
use std::sync::Arc;

#[tokio::test]
#[ignore = "requires disposable MinIO integration server"]
async fn live_minio_pair_two_local_cores_encrypted_invite_new_files_both_directions() {
    let Ok(endpoint) = std::env::var("AQ_S3_TEST_ENDPOINT") else { return; };
    assert!(endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://localhost:"));
    let id = std::env::var("AWS_ACCESS_KEY_ID").expect("CI-only MinIO access ID");
    let secret = std::env::var("AWS_SECRET_ACCESS_KEY").expect("CI-only MinIO secret");
    let temp = tempfile::tempdir().unwrap();
    let root_a = temp.path().join("a-notes");
    let root_b = temp.path().join("b-notes");
    let app_a = temp.path().join("a-data");
    let app_b = temp.path().join("b-data");
    for dir in [&root_a, &root_b, &app_a, &app_b] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(root_a.join("A.md"), b"the first new note from A").unwrap();
    fs::write(root_b.join("B.md"), b"the independent new note from B").unwrap();
    let first = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
    let second = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
    first.ui_state.resolve_workspace(root_a.to_str().unwrap(), 1).unwrap();
    second.ui_state.resolve_workspace(root_b.to_str().unwrap(), 2).unwrap();
    let local_a = first.ui_state.list_workspaces().unwrap()[0].id;
    let local_b = second.ui_state.list_workspaces().unwrap()[0].id;
    assert_ne!(local_a, local_b);

    let vault = VaultId::new();
    assert_ne!(vault.as_uuid(), local_a);
    assert_ne!(vault.as_uuid(), local_b);
    let profile_s3 = S3ConnectionProfile {
        bucket: "akula-pair-e2e-test".into(),
        region: "us-east-1".into(),
        endpoint: Some(endpoint.clone()),
        force_path_style: true,
    };
    let ns = S3Keyspace::new("akula-sync/pair-ci", vault).unwrap();
    let network = AwsS3ProbeClient::connect_with_explicit_credentials(
        profile_s3.clone(), ns.clone(), &id, &secret,
    ).await.unwrap();
    network.test_ensure_minio_bucket().await.unwrap();
    assert_eq!(network.probe_pair_read_write().await.unwrap(), S3PairAccess::ReadWrite);
    let mut j = S3HeadJournal::open(&app_a.join("s3-head.sqlite")).unwrap();
    let genesis = bootstrap_new_remote_vault(&network, &ns, &mut j).await.unwrap();
    drop(j);
    let profile = S3PairProfile {
        provider: "minio".into(), endpoint: Some(endpoint),
        region: "us-east-1".into(), bucket: profile_s3.bucket.clone(),
        prefix: "akula-sync/pair-ci".into(), cloud_vault_id: vault,
        genesis_fingerprint: genesis.genesis_fingerprint,
    };
    store_new_pair_profile(&root_a, &app_a, &profile).unwrap();
    let invitation = profile.with_keys(&id, &secret).unwrap();
    let cipher = seal_s3_pair_invite(&invitation, "correct long pairing password").unwrap();
    assert!(!cipher.windows(secret.len()).any(|piece| piece == secret.as_bytes()));
    let received = open_s3_pair_invite(&cipher, "correct long pairing password").unwrap();
    let imported = S3PairProfile::from_authenticated_invite(&received);
    assert_eq!(imported, profile);
    accept_second_pair_profile(
        &second, &root_b, &app_b, &imported, &received.access_key_id,
        &received.secret_access_key,
    ).await.unwrap();

    // The pre-existing lock scope and physical identity are provisioned by
    // the trusted test host OUTSIDE both vaults (a real Windows installer must
    // independently prove this same contract; don't auto-create the scope).
    let make_custody = |core: &Core, root: &Path, appdata: &Path| {
        let pinned = PinnedProbeRoot::open(root).unwrap();
        let scope = temp.path().join("external-filegate")
            .join(scoped_leaf(pinned.identity()));
        fs::create_dir_all(&scope).unwrap();
        let lock = scope.join(LOCK_NAME);
        fs::write(&lock, b"host-owned preexisting lock").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&scope, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let scope = fs::canonicalize(&scope).unwrap();
        let admission = RegisteredReadAdmission::open(core, root).unwrap();
        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("s3-profiles.sqlite"),
        ).unwrap();
        assert!(bindings.load_for_registered(&admission).unwrap().is_some());
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &appdata.join("s3-custody.sqlite"),
        ).unwrap();
        custody.bind_registered_s3(&admission, &bindings, &scope).unwrap();
        scope
    };

    // Publish A before provisioning its local File Gate receiver; publishing
    // does not rely on destination lock custody.
    let published_a = sync_new_files_once(
        &first, &root_a, &app_a, None, &profile, &id, &secret,
    ).await.unwrap();
    assert_eq!(published_a.uploaded_new, 1);
    assert_eq!(published_a.received_unconfirmed, 0);

    let scope_b = make_custody(&second, &root_b, &app_b);
    let published_b = sync_new_files_once(
        &second, &root_b, &app_b, Some(&scope_b), &imported, &id, &secret,
    ).await.unwrap();
    assert_eq!(published_b.uploaded_new, 1);
    assert_eq!(published_b.received_unconfirmed, 1);
    assert_eq!(fs::read(root_b.join("A.md")).unwrap(), b"the first new note from A");
    assert_eq!(fs::read(root_b.join("B.md")).unwrap(), b"the independent new note from B");

    // The first Core's local S3 binding must be provisioned from an
    // independently verified root before testing its receive-side custody.
    let verified_a = verify_invited_genesis(
        &network, &ns, &profile.genesis_fingerprint,
    ).await.unwrap();
    let admission_a = RegisteredReadAdmission::open(&first, &root_a).unwrap();
    let mut binding_a = SqliteRegisteredS3Bindings::open(
        &admission_a, &app_a.join("s3-profiles.sqlite"),
    ).unwrap();
    binding_a.bind_existing_verified_s3(
        &admission_a, "akula-pair-v1", &profile_s3, &ns, &verified_a,
    ).unwrap();
    drop(binding_a);
    drop(admission_a);
    let scope_a = make_custody(&first, &root_a, &app_a);
    let received_a = sync_new_files_once(
        &first, &root_a, &app_a, Some(&scope_a), &profile, &id, &secret,
    ).await.unwrap();
    assert_eq!(received_a.received_unconfirmed, 1);
    assert_eq!(fs::read(root_a.join("B.md")).unwrap(), b"the independent new note from B");

    drop(first);
    drop(second);
    assert_eq!(fs::read(root_a.join("A.md")).unwrap(), fs::read(root_b.join("A.md")).unwrap());
    assert_eq!(fs::read(root_a.join("B.md")).unwrap(), fs::read(root_b.join("B.md")).unwrap());
}
