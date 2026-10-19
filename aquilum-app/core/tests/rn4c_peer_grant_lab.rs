//! RN-4C: disabled-by-default Core-registered grant ledger tests.
//! No P2P socket, File Gate write, directory publishing or trusted host claim.
#![cfg(feature = "staya-core-grant-lab")]

use aquilum_core::sync::{
    peer_grant_lab::{ApprovedPull, GrantLedgerError, RegisteredPullGrantLedger},
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
        vault_id, device_id,
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
    assert!(!db.exists(), "read-only admission must never bootstrap grants");

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
    let issued = first.issue(p).unwrap();
    assert_eq!(issued.epoch, 1);
    assert_eq!(issued.vault_id, vault);
    assert_eq!(issued.device_id, device);
    assert_eq!(first.read_active(vault, device, 10).unwrap(), issued);
    assert!(matches!(first.issue(p), Err(GrantLedgerError::ActiveGrantExists)));

    // A second Core/SQLite connection sees exactly the same committed epoch.
    let cold = registered_core(&app, &root);
    let mut second = RegisteredPullGrantLedger::open_existing(&cold, &root, &db).unwrap();
    assert_eq!(second.read_active(vault, device, 10).unwrap(), issued);
    assert_eq!(second.revoke(vault, device, 1).unwrap(), 2);
    assert!(matches!(first.read_active(vault, device, 10), Err(GrantLedgerError::Revoked)));
    assert!(matches!(first.revoke(vault, device, 1), Err(GrantLedgerError::StaleEpoch)));
    drop(first);
    drop(second);
    drop(cold);

    // No automatic re-enrollment: tombstone is durable, not an empty roster.
    let recovered = registered_core(&app, &root);
    let mut last = RegisteredPullGrantLedger::open_existing(&recovered, &root, &db).unwrap();
    assert!(matches!(last.read_active(vault, device, 10), Err(GrantLedgerError::Revoked)));
    let renewal = last.issue(p).unwrap();
    assert_eq!(renewal.epoch, 3, "revoked generation must not be reused");
    assert!(matches!(last.revoke(vault, device, 1), Err(GrantLedgerError::StaleEpoch)));
    assert_eq!(last.read_active(vault, device, 10).unwrap(), renewal);
    assert!(fs::read_dir(&root).unwrap().next().is_none(), "no user-vault mutation");
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
    let device = DeviceId::new();
    first.issue(proposal(vault_a, device)).unwrap();

    assert!(matches!(
        first.read_active(VaultId::new(), device, 10),
        Err(GrantLedgerError::MissingGrant)
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
    let d = DeviceId::new();
    let mut p = proposal(v, d);
    p.object_length = 4 * 1024 * 1024 + 1;
    assert!(matches!(ledger.issue(p), Err(GrantLedgerError::InvalidProposal)));
    assert!(matches!(ledger.read_active(v, d, 100), Err(GrantLedgerError::MissingGrant)));
    p.object_length = 200;
    let granted = ledger.issue(p).unwrap();
    assert_eq!(granted.epoch, 1);
    assert!(matches!(ledger.read_active(v, d, 1000), Err(GrantLedgerError::InvalidProposal)));
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}
