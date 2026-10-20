//! RN-4E1b-03a: a read-only, DOUBLE-registry correlation falsifier.
//! This entire module is compiled ONLY under cfg(test) and two default-OFF
//! features. No network endpoint, File Gate write, receipt or APPLIED exists.
//! The "remote_id" and OS lock identities below are synthetic lab fixtures.

use super::host_custody_registry::{
    HostCustodyError, HostCustodyRecord, PhysicalObjectId, SharedHostCustodyRegistry,
};
use super::peer_grant_lab::{
    ApprovedPull, GrantLedgerError, RegisteredPullGrantLedger, StoredPull,
};
use super::{DeviceId, RegisteredReadAdmission, VaultId};
use crate::{Core, CoreEvent};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;

#[allow(dead_code)]
#[derive(Debug)]
enum CheckError {
    Host(HostCustodyError),
    Grant(GrantLedgerError),
    ScopeChanged,
    WrongRemote,
}
impl From<HostCustodyError> for CheckError {
    fn from(v: HostCustodyError) -> Self { Self::Host(v) }
}
impl From<GrantLedgerError> for CheckError {
    fn from(v: GrantLedgerError) -> Self { Self::Grant(v) }
}

/// Two independently readable stores are NOT one atomic trust transaction.
/// This deliberately returns only (), not a capability or policy permit.
/// A hostile process may replace an unprivileged host database, so even a
/// positive result is never sufficient to publish a user file.
fn observe(
    host: &SharedHostCustodyRegistry,
    grants: &RegisteredPullGrantLedger<'_>,
    selected_core: &Core,
    selected_root: &Path,
    host_record: &HostCustodyRecord,
    expected: StoredPull,
    synthetic_remote_id: [u8; 32],
    now: u64,
) -> Result<(), CheckError> {
    grants.recheck_selected_replica_for_lab(
        selected_core, selected_root, expected.vault_id
    )?;
    let selected = RegisteredReadAdmission::open(selected_core, selected_root)
        .map_err(GrantLedgerError::Unregistered)?;
    let (device, file) = selected
        .registered_root_identity_for_profile()
        .map_err(GrantLedgerError::Unregistered)?;
    let id = PhysicalObjectId { device, file };

    host.inspect_exact_replica_scope(
        selected.registered_root_path(), id, expected.vault_id,
        &host_record.profile_id, host_record.lock_identity,
    )?;
    let actual = grants.read_active(expected.vault_id, expected.device_id, now)?;
    if actual != expected { return Err(CheckError::ScopeChanged); }
    if actual.authenticated_endpoint != synthetic_remote_id {
        return Err(CheckError::WrongRemote);
    }
    // Sequential rechecks detect common changes, not concurrent malicious
    // swaps or a revoke committed after return. NO WRITE may follow this.
    grants.recheck_selected_replica_for_lab(
        selected_core, selected_root, expected.vault_id
    )?;
    host.inspect_exact_replica_scope(
        selected.registered_root_path(), id, expected.vault_id,
        &host_record.profile_id, host_record.lock_identity,
    )?;
    Ok(())
}

struct Fixture {
    temp: TempDir,
    core_a: Arc<Core>,
    core_b: Arc<Core>,
    root_a: PathBuf,
    root_b: PathBuf,
    app_a: PathBuf,
    app_b: PathBuf,
    host_db: PathBuf,
    host: SharedHostCustodyRegistry,
    record_a: HostCustodyRecord,
    record_b: HostCustodyRecord,
    vault: VaultId,
    peer: DeviceId,
}
fn core(app: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(app).unwrap();
    fs::create_dir_all(root).unwrap();
    let core = Core::open(app, Arc::new(|_: CoreEvent| {}));
    core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
    core
}
fn record(core: &Core, root: &Path, scope: &Path,
          vault: VaultId, profile: &str, lock_file: u64) -> HostCustodyRecord {
    let pinned = RegisteredReadAdmission::open(core, root).unwrap();
    let (device, file) = pinned.registered_root_identity_for_profile().unwrap();
    HostCustodyRecord {
        root_path: fs::canonicalize(root).unwrap().to_string_lossy().into_owned(),
        root_identity: PhysicalObjectId { device, file },
        vault_id: vault,
        profile_id: profile.into(),
        custody_scope_path: fs::canonicalize(scope).unwrap().to_string_lossy().into_owned(),
        // Only a synthetic marker, NOT a pinned OS/Broker lock handle.
        lock_identity: PhysicalObjectId { device, file: lock_file },
    }
}
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let app_a = temp.path().join("app-a");
    let app_b = temp.path().join("app-b");
    let root_a = temp.path().join("root-a");
    let root_b = temp.path().join("root-b");
    let host_dir = temp.path().join("host");
    let scope_a = temp.path().join("lock-a");
    let scope_b = temp.path().join("lock-b");
    for dir in [&host_dir, &scope_a, &scope_b] {
        fs::create_dir_all(dir).unwrap();
    }
    let core_a = core(&app_a, &root_a);
    let core_b = core(&app_b, &root_b);
    let vault = VaultId::new();
    let peer = DeviceId::new();
    let record_a = record(&core_a, &root_a, &scope_a, vault, "replica-a", 1001);
    let record_b = record(&core_b, &root_b, &scope_b, vault, "replica-b", 1002);
    let host_db = host_dir.join("authority.sqlite3");
    let mut host = SharedHostCustodyRegistry::provision_new(
        &host_db, &[&app_a, &app_b, &root_a, &root_b]
    ).unwrap();
    host.enroll(&record_a).unwrap();
    host.enroll(&record_b).unwrap();
    Fixture { temp, core_a, core_b, root_a, root_b, app_a, app_b,
              host_db, host, record_a, record_b, vault, peer }
}
fn grant(core: &Core, root: &Path, db: &Path, vault: VaultId,
         device: DeviceId, marker: u8) -> (RegisteredPullGrantLedger<'_>, StoredPull) {
    let mut grants = RegisteredPullGrantLedger::provision_new(core, root, db).unwrap();
    grants.bind_vault_once(vault).unwrap();
    let current = grants.issue(ApprovedPull {
        vault_id: vault,
        device_id: device,
        authenticated_endpoint: [7; 32],
        trusted_issuer: [8; 32],
        object_blake3: [marker; 32],
        object_length: 128,
        expires_at: 1_000,
        nonce: [marker; 16],
    }).unwrap();
    (grants, current)
}
fn roots_unchanged(f: &Fixture) {
    assert!(fs::read_dir(&f.root_a).unwrap().next().is_none());
    assert!(fs::read_dir(&f.root_b).unwrap().next().is_none());
}

#[test]
fn same_vault_on_two_registered_roots_never_allows_cross_replica_grant() {
    let f = fixture();
    let (a, ga) = grant(&f.core_a, &f.root_a, &f.app_a.join("grants.sqlite3"),
                        f.vault, f.peer, 11);
    let (b, gb) = grant(&f.core_b, &f.root_b, &f.app_b.join("grants.sqlite3"),
                        f.vault, f.peer, 22);
    assert!(observe(&f.host, &a, &f.core_a, &f.root_a, &f.record_a, ga, [7; 32], 100).is_ok());
    assert!(observe(&f.host, &b, &f.core_b, &f.root_b, &f.record_b, gb, [7; 32], 100).is_ok());
    // VaultId, DeviceId, and EndpointId are identical for A and B.
    assert!(matches!(
        observe(&f.host, &a, &f.core_b, &f.root_b, &f.record_b, ga, [7; 32], 100),
        Err(CheckError::Grant(GrantLedgerError::RootMismatch))
    ));
    assert!(matches!(
        observe(&f.host, &b, &f.core_a, &f.root_a, &f.record_a, gb, [7; 32], 100),
        Err(CheckError::Grant(GrantLedgerError::RootMismatch))
    ));
    assert!(observe(&f.host, &a, &f.core_a, &f.root_a,
                    &f.record_b, ga, [7; 32], 100).is_err());
    roots_unchanged(&f);
}

#[test]
fn wrong_peer_nonce_expiry_and_revoke_are_denied_without_file_mutation() {
    let f = fixture();
    let (mut a, ga) = grant(&f.core_a, &f.root_a, &f.app_a.join("grants.sqlite3"),
                            f.vault, f.peer, 11);
    let mut impostor = ga;
    impostor.device_id = DeviceId::new();
    assert!(observe(&f.host, &a, &f.core_a, &f.root_a,
                    &f.record_a, impostor, [7; 32], 100).is_err());
    impostor = ga;
    impostor.nonce = [44; 16];
    assert!(matches!(
        observe(&f.host, &a, &f.core_a, &f.root_a,
                &f.record_a, impostor, [7; 32], 100),
        Err(CheckError::ScopeChanged)
    ));
    assert!(matches!(
        observe(&f.host, &a, &f.core_a, &f.root_a,
                &f.record_a, ga, [90; 32], 100),
        Err(CheckError::WrongRemote)
    ));
    assert!(observe(&f.host, &a, &f.core_a, &f.root_a,
                    &f.record_a, ga, [7; 32], ga.expires_at).is_err());
    assert_eq!(a.revoke(f.vault, f.peer, 1).unwrap(), 2);
    assert!(observe(&f.host, &a, &f.core_a, &f.root_a,
                    &f.record_a, ga, [7; 32], 100).is_err());
    roots_unchanged(&f);
}

#[test]
fn cold_reopen_keeps_replica_scopes_distinct() {
    let f = fixture();
    let (a, ga) = grant(&f.core_a, &f.root_a, &f.app_a.join("grants.sqlite3"),
                        f.vault, f.peer, 11);
    drop(a);
    let fresh_core = core(&f.app_a, &f.root_a);
    let fresh_grants = RegisteredPullGrantLedger::open_existing(
        &fresh_core, &f.root_a, &f.app_a.join("grants.sqlite3")
    ).unwrap();
    let fresh_host = SharedHostCustodyRegistry::open_existing(&f.host_db).unwrap();
    assert!(observe(&fresh_host, &fresh_grants, &fresh_core, &f.root_a,
                    &f.record_a, ga, [7; 32], 100).is_ok());
    assert!(observe(&fresh_host, &fresh_grants, &f.core_b, &f.root_b,
                    &f.record_b, ga, [7; 32], 100).is_err());
    roots_unchanged(&f);
}

#[test]
fn forged_host_database_is_a_positive_counterexample_to_installed_trust() {
    let f = fixture();
    let (a, ga) = grant(&f.core_a, &f.root_a, &f.app_a.join("grants.sqlite3"),
                        f.vault, f.peer, 11);
    let fake_dir = f.temp.path().join("fake-host");
    fs::create_dir(&fake_dir).unwrap();
    let mut fake = SharedHostCustodyRegistry::provision_new(
        &fake_dir.join("host.sqlite3"), &[&f.root_a, &f.app_a]
    ).unwrap();
    fake.enroll(&f.record_a).unwrap();
    // This POSITIVE result demonstrates the security gap, not a write
    // permission: ordinary SQLite + valid BLAKE3 cannot prove installer
    // custody. RN-4E1b-03b requires real OS owner/Broker verification.
    assert!(observe(&fake, &a, &f.core_a, &f.root_a,
                    &f.record_a, ga, [7; 32], 100).is_ok());
    roots_unchanged(&f);
}
