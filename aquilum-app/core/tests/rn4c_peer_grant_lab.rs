//! RN-4C: disabled-by-default Core-registered grant ledger tests.
//! No P2P socket, File Gate write, directory publishing or trusted host claim.
#![cfg(feature = "staya-core-grant-lab")]

use aquilum_core::sync::{
    peer_grant_lab::{ApprovedPull, GrantLedgerError, RegisteredPullGrantLedger, StoredPull},
    DeviceId, VaultId,
};
use aquilum_core::{Core, CoreEvent};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn registered_core(app: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(root).unwrap();
    fs::create_dir_all(app).unwrap();
    let core = Core::open(app, Arc::new(|_: CoreEvent| {}));
    core.ui_state
        .resolve_workspace(root.to_str().expect("utf-8 temp workspace"), 1)
        .expect("Core-owned registered workspace");
    core
}

fn proposal(vault_id: VaultId, device_id: DeviceId) -> ApprovedPull {
    ApprovedPull {
        vault_id,
        device_id,
        authenticated_endpoint: [7; 32],
        trusted_issuer: [8; 32],
        object_blake3: [9; 32],
        object_length: 4096,
        expires_at: 1_000,
        nonce: [10; 16],
    }
}

#[test]
fn missing_or_unregistered_ledger_does_not_bootstrap_from_network() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("staya-lab.sqlite3");
    let core = registered_core(&app, &root);
    assert!(matches!(
        RegisteredPullGrantLedger::open_existing(&core, &root, &db),
        Err(GrantLedgerError::MissingStore)
    ));
    assert!(
        !db.exists(),
        "read-only admission must never bootstrap grants"
    );

    let other_root = t.path().join("unregistered-vault");
    fs::create_dir(&other_root).unwrap();
    assert!(matches!(
        RegisteredPullGrantLedger::provision_new(&core, &other_root, &db),
        Err(GrantLedgerError::Unregistered(_))
    ));
    assert!(!db.exists());
}

#[test]
fn core_registered_grant_revocation_survives_cold_restart_and_reissue() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("staya-lab.sqlite3");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let device = DeviceId::new();
    let p = proposal(vault, device);

    let mut first = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    first.bind_vault_once(vault).unwrap();
    let issued = first.issue(p).unwrap();
    assert_eq!(issued.epoch, 1);
    assert_eq!(issued.vault_id, vault);
    assert_eq!(issued.device_id, device);
    assert_eq!(first.read_active(vault, device, 10).unwrap(), issued);
    assert!(matches!(
        first.issue(p),
        Err(GrantLedgerError::ActiveGrantExists)
    ));

    // A second Core/SQLite connection sees exactly the same committed epoch.
    let cold = registered_core(&app, &root);
    let mut second = RegisteredPullGrantLedger::open_existing(&cold, &root, &db).unwrap();
    assert_eq!(second.read_active(vault, device, 10).unwrap(), issued);
    assert_eq!(second.revoke(vault, device, 1).unwrap(), 2);
    assert!(matches!(
        first.read_active(vault, device, 10),
        Err(GrantLedgerError::Revoked)
    ));
    assert!(matches!(
        first.revoke(vault, device, 1),
        Err(GrantLedgerError::StaleEpoch)
    ));
    drop(first);
    drop(second);
    drop(cold);

    // No automatic re-enrollment: tombstone is durable, not an empty roster.
    let recovered = registered_core(&app, &root);
    let mut last = RegisteredPullGrantLedger::open_existing(&recovered, &root, &db).unwrap();
    assert!(matches!(
        last.read_active(vault, device, 10),
        Err(GrantLedgerError::Revoked)
    ));
    let renewal = last.issue(p).unwrap();
    assert_eq!(renewal.epoch, 3, "revoked generation must not be reused");
    assert!(matches!(
        last.revoke(vault, device, 1),
        Err(GrantLedgerError::StaleEpoch)
    ));
    assert_eq!(last.read_active(vault, device, 10).unwrap(), renewal);
    assert!(
        fs::read_dir(&root).unwrap().next().is_none(),
        "no user-vault mutation"
    );
}

#[test]
fn binding_is_workspace_and_physical_root_specific() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root_a = t.path().join("vault-a");
    let root_b = t.path().join("vault-b");
    let db = app.join("staya-lab.sqlite3");
    let core = registered_core(&app, &root_a);
    let mut first = RegisteredPullGrantLedger::provision_new(&core, &root_a, &db).unwrap();
    let vault_a = VaultId::new();
    first.bind_vault_once(vault_a).unwrap();
    let device = DeviceId::new();
    first.issue(proposal(vault_a, device)).unwrap();

    assert!(matches!(
        first.read_active(VaultId::new(), device, 10),
        Err(GrantLedgerError::ForeignVault)
    ));
    let other = registered_core(&app, &root_b);
    assert!(matches!(
        RegisteredPullGrantLedger::open_existing(&other, &root_b, &db),
        Err(GrantLedgerError::RootMismatch)
    ));
    assert!(matches!(
        RegisteredPullGrantLedger::provision_new(&core, &root_a, &db),
        Err(GrantLedgerError::Io(_))
    ));

    let within_vault: PathBuf = root_a.join("not-appdata.sqlite3");
    assert!(matches!(
        RegisteredPullGrantLedger::provision_new(&core, &root_a, &within_vault),
        Err(GrantLedgerError::UnsafeStore)
    ));
    assert!(!within_vault.exists());
}

#[test]
fn oversize_or_expired_grants_deny_before_any_file_operations() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("staya-lab.sqlite3");
    let core = registered_core(&app, &root);
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let v = VaultId::new();
    ledger.bind_vault_once(v).unwrap();
    let d = DeviceId::new();
    let mut p = proposal(v, d);
    p.object_length = 4 * 1024 * 1024 + 1;
    assert!(matches!(
        ledger.issue(p),
        Err(GrantLedgerError::InvalidProposal)
    ));
    assert!(matches!(
        ledger.read_active(v, d, 100),
        Err(GrantLedgerError::MissingGrant)
    ));
    p.object_length = 200;
    let granted = ledger.issue(p).unwrap();
    assert_eq!(granted.epoch, 1);
    assert!(matches!(
        ledger.read_active(v, d, 1000),
        Err(GrantLedgerError::InvalidProposal)
    ));
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn missing_vault_binding_never_grants_or_implicitly_enrolls() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("vault-binding.sqlite3");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let device = DeviceId::new();
    let p = proposal(vault, device);
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    assert!(matches!(
        ledger.issue(p),
        Err(GrantLedgerError::MissingVaultBinding)
    ));
    assert!(matches!(
        ledger.read_active(vault, device, 10),
        Err(GrantLedgerError::MissingVaultBinding)
    ));
    assert!(matches!(
        ledger.revoke(vault, device, 1),
        Err(GrantLedgerError::MissingVaultBinding)
    ));
    let fabricated = StoredPull {
        vault_id: p.vault_id,
        device_id: p.device_id,
        authenticated_endpoint: p.authenticated_endpoint,
        trusted_issuer: p.trusted_issuer,
        object_blake3: p.object_blake3,
        object_length: p.object_length,
        epoch: 1,
        expires_at: p.expires_at,
        nonce: p.nonce,
    };
    assert!(matches!(
        ledger.with_fenced_pull_preflight(fabricated, [7; 32], 10, |_| {
            panic!("unbound vault must never enter the callback");
        }),
        Err(GrantLedgerError::MissingVaultBinding)
    ));
    drop(ledger);
    let reopened = RegisteredPullGrantLedger::open_existing(&core, &root, &db).unwrap();
    assert!(matches!(
        reopened.read_active(vault, device, 10),
        Err(GrantLedgerError::MissingVaultBinding)
    ));
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn one_time_binding_persists_after_restart_and_rejects_foreign_vault() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("vault-binding.sqlite3");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let foreign = VaultId::new();
    let device = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    ledger.bind_vault_once(vault).unwrap();
    assert!(matches!(
        ledger.bind_vault_once(vault),
        Err(GrantLedgerError::VaultAlreadyBound)
    ));
    assert!(matches!(
        ledger.bind_vault_once(foreign),
        Err(GrantLedgerError::VaultAlreadyBound)
    ));
    assert!(matches!(
        ledger.issue(proposal(foreign, device)),
        Err(GrantLedgerError::ForeignVault)
    ));
    let issued = ledger.issue(proposal(vault, device)).unwrap();
    drop(ledger);
    let cold = registered_core(&app, &root);
    let mut reopened = RegisteredPullGrantLedger::open_existing(&cold, &root, &db).unwrap();
    assert_eq!(reopened.read_active(vault, device, 10).unwrap(), issued);
    assert!(matches!(
        reopened.read_active(foreign, device, 10),
        Err(GrantLedgerError::ForeignVault)
    ));
    assert!(matches!(
        reopened.revoke(foreign, device, 1),
        Err(GrantLedgerError::ForeignVault)
    ));
    let mut forged = issued;
    forged.vault_id = foreign;
    assert!(matches!(
        reopened.with_fenced_pull_preflight(forged, [7; 32], 10, |_| {
            panic!("foreign vault must not execute a callback");
        }),
        Err(GrantLedgerError::ForeignVault)
    ));
    assert_eq!(reopened.revoke(vault, device, 1).unwrap(), 2);
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn modified_binding_identity_or_generation_denies_grants() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("vault-binding.sqlite3");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let device = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    ledger.bind_vault_once(vault).unwrap();
    let issued = ledger.issue(proposal(vault, device)).unwrap();
    let other = rusqlite::Connection::open(&db).unwrap();
    other.execute(
        "UPDATE vault_binding SET generation=2 WHERE singleton=1",
        [],
    ).unwrap();
    assert!(matches!(
        ledger.read_active(vault, device, 10),
        Err(GrantLedgerError::CorruptRow)
    ));
    other.execute(
        "UPDATE vault_binding SET generation=1, root_ino='invalid'
         WHERE singleton=1",
        [],
    ).unwrap();
    assert!(matches!(
        ledger.with_fenced_pull_preflight(issued, [7; 32], 10, |_| {
            panic!("incorrect physical identity must deny callback");
        }),
        Err(GrantLedgerError::RootMismatch)
    ));
    other.execute("DELETE FROM vault_binding WHERE singleton=1", []).unwrap();
    assert!(matches!(
        ledger.read_active(vault, device, 10),
        Err(GrantLedgerError::MissingVaultBinding)
    ));
    assert!(matches!(
        ledger.bind_vault_once(vault),
        Err(GrantLedgerError::VaultAlreadyBound)
    ));
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn legacy_schema_is_not_silently_migrated_to_bound_policy() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root = t.path().join("vault");
    let db = app.join("legacy-ledger.sqlite3");
    let core = registered_core(&app, &root);
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute_batch("PRAGMA user_version=1;").unwrap();
    drop(connection);
    assert!(matches!(
        RegisteredPullGrantLedger::open_existing(&core, &root, &db),
        Err(GrantLedgerError::InvalidSchema)
    ));
    let unchanged = rusqlite::Connection::open(&db).unwrap();
    let version: i64 = unchanged
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 1);
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn different_registered_workspace_cannot_use_this_bound_store() {
    let t = tempfile::tempdir().unwrap();
    let app = t.path().join("appdata");
    let root_a = t.path().join("vault-a");
    let root_b = t.path().join("vault-b");
    let db = app.join("vault-binding.sqlite3");
    let core = registered_core(&app, &root_a);
    let vault = VaultId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root_a, &db).unwrap();
    ledger.bind_vault_once(vault).unwrap();
    drop(ledger);
    let other = registered_core(&app, &root_b);
    assert!(matches!(
        RegisteredPullGrantLedger::open_existing(&other, &root_b, &db),
        Err(GrantLedgerError::RootMismatch)
    ));
    assert!(fs::read_dir(&root_a).unwrap().next().is_none());
    assert!(fs::read_dir(&root_b).unwrap().next().is_none());
}
