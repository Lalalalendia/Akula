//! RN-4D: opt-in revocation-fence lab, not a production File Gate permit.
#![cfg(feature = "staya-core-grant-lab")]

use aquilum_core::sync::{
    peer_grant_lab::{ApprovedPull, GrantLedgerError, RegisteredPullGrantLedger},
    DeviceId, VaultId,
};
use aquilum_core::{Core, CoreEvent};
use rusqlite::{params, Connection, ErrorCode};
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::sync::Arc;

fn registered_core(app: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(app).unwrap();
    fs::create_dir_all(root).unwrap();
    let core = Core::open(app, Arc::new(|_: CoreEvent| {}));
    core.ui_state
        .resolve_workspace(root.to_str().unwrap(), 1)
        .expect("trusted local workspace registration");
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
        expires_at: 1000,
        nonce: [10; 16],
    }
}

#[test]
fn immediate_guard_blocks_second_sqlite_writer_until_probe_completes() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("appdata");
    let root = tmp.path().join("vault");
    let db = app.join("grants.sqlite3");
    let core = registered_core(&app, &root);
    let v = VaultId::new();
    let d = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let original = ledger.issue(proposal(v, d)).unwrap();

    let mut probe_was_called = false;
    ledger
        .with_fenced_pull_preflight(original, [7; 32], 100, |inside| {
            assert_eq!(*inside, original);
            probe_was_called = true;

            // This is a SEPARATE SQLite connection on the same file, not an
            // application-scoped Mutex. A concurrent revoke cannot commit
            // between fenced scope revalidation and the end of the probe.
            let competitor = Connection::open(&db).unwrap();
            competitor.busy_timeout(std::time::Duration::ZERO).unwrap();
            let update = competitor.execute(
                "UPDATE pull_grants SET epoch=2,active=0
                 WHERE vault=?1 AND device=?2 AND epoch=1 AND active=1",
                params![v.to_string(), d.to_string()],
            );
            assert!(
                matches!(update, Err(rusqlite::Error::SqliteFailure(ref error, _))
                    if error.code == ErrorCode::DatabaseBusy),
                "the competing SQLite revocation must NOT commit while the preflight fence is held"
            );
            Ok(())
        })
        .unwrap();
    assert!(probe_was_called);
    assert_eq!(ledger.read_active(v, d, 100).unwrap(), original);

    // As soon as the fence is released, the same revocation is permitted.
    assert_eq!(ledger.revoke(v, d, 1).unwrap(), 2);
    assert!(matches!(
        ledger.with_fenced_pull_preflight(original, [7; 32], 100, |_| {
            panic!("revoked scope must never reach a callback")
        }),
        Err(GrantLedgerError::Revoked)
    ));
    let reissued = ledger.issue(proposal(v, d)).unwrap();
    assert_eq!(reissued.epoch, 3);
    assert!(matches!(
        ledger.with_fenced_pull_preflight(original, [7; 32], 100, |_| {
            panic!("stale snapshot must never reach callback")
        }),
        Err(GrantLedgerError::StaleEpoch)
    ));
    ledger
        .with_fenced_pull_preflight(reissued, [7; 32], 100, |_| Ok(()))
        .unwrap();
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn wrong_endpoint_or_signed_scope_mismatch_denies_before_probe() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("appdata");
    let root = tmp.path().join("vault");
    let db = app.join("grants.sqlite3");
    let core = registered_core(&app, &root);
    let v = VaultId::new();
    let d = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let expected = ledger.issue(proposal(v, d)).unwrap();
    let calls = Cell::new(0);

    let mut wrong_issuer = expected;
    wrong_issuer.trusted_issuer = [2; 32];
    let mut wrong_digest = expected;
    wrong_digest.object_blake3 = [3; 32];
    let mut wrong_nonce = expected;
    wrong_nonce.nonce = [4; 16];
    let mut wrong_vault = expected;
    wrong_vault.vault_id = VaultId::new();
    let mut wrong_len = expected;
    wrong_len.object_length += 1;

    for mismatch in [wrong_issuer, wrong_digest, wrong_nonce, wrong_len] {
        assert!(matches!(
            ledger.with_fenced_pull_preflight(mismatch, [7; 32], 100, |_| {
                calls.set(calls.get() + 1);
                Ok(())
            }),
            Err(GrantLedgerError::ScopeChanged)
        ));
    }
    assert!(matches!(
        ledger.with_fenced_pull_preflight(wrong_vault, [7; 32], 100, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        }),
        Err(GrantLedgerError::MissingGrant)
    ));
    assert!(matches!(
        ledger.with_fenced_pull_preflight(expected, [99; 32], 100, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        }),
        Err(GrantLedgerError::WrongAuthenticatedEndpoint)
    ));
    assert!(matches!(
        ledger.with_fenced_pull_preflight(expected, [7; 32], 1000, |_| {
            calls.set(calls.get() + 1);
            Ok(())
        }),
        Err(GrantLedgerError::InvalidProposal)
    ));
    assert_eq!(calls.get(), 0);
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn failed_read_only_probe_rolls_back_and_cannot_fence_later_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("appdata");
    let root = tmp.path().join("vault");
    let db = app.join("grants.sqlite3");
    let core = registered_core(&app, &root);
    let v = VaultId::new();
    let d = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let expected = ledger.issue(proposal(v, d)).unwrap();

    assert!(matches!(
        ledger.with_fenced_pull_preflight(expected, [7; 32], 100, |_| {
            Err(GrantLedgerError::InvalidProposal)
        }),
        Err(GrantLedgerError::InvalidProposal)
    ));
    assert_eq!(ledger.read_active(v, d, 100).unwrap(), expected);
    assert_eq!(ledger.revoke(v, d, 1).unwrap(), 2);
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}
