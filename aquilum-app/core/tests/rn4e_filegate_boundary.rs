//! RN-4E0: disposable, opt-in interprocess experiment at the *real* File Gate.
//!
//! This test deliberately does NOT install a P2P writer, grant an application
//! capability, issue an ACK, or mark any sync baseline APPLIED. The existing
//! arbitrary preflight callback is NOT a safe product publication API.
#![cfg(feature = "staya-core-grant-lab")]

use aquilum_core::files::gate;
use aquilum_core::history::Source;
use aquilum_core::sync::{
    peer_grant_lab::{ApprovedPull, GrantLedgerError, RegisteredPullGrantLedger, StoredPull},
    DeviceId, VaultId,
};
use aquilum_core::{Core, CoreEvent};
use rusqlite::{params, Connection, ErrorCode};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const CONTENT: &str = "RN-4E0 verified local fixture - not network payload\n";
const NOW: u64 = 100;

fn registered_core(app: &Path, root: &Path) -> Arc<Core> {
    fs::create_dir_all(app).unwrap();
    fs::create_dir_all(root).unwrap();
    let core = Core::open(app, Arc::new(|_: CoreEvent| {}));
    core.ui_state
        .resolve_workspace(root.to_str().unwrap(), 1)
        .expect("workspace must be explicitly registered");
    core
}

fn proposal(vault_id: VaultId, device_id: DeviceId) -> ApprovedPull {
    ApprovedPull {
        vault_id,
        device_id,
        authenticated_endpoint: [7; 32],
        trusted_issuer: [8; 32],
        object_blake3: *blake3::hash(CONTENT.as_bytes()).as_bytes(),
        object_length: CONTENT.len() as u64,
        expires_at: 1_000,
        nonce: [10; 16],
    }
}

fn spawn_competing_process(
    mode: &str,
    app: &Path,
    root: &Path,
    db: &Path,
    vault: VaultId,
    device: DeviceId,
    result_file: &Path,
) -> Child {
    Command::new(std::env::current_exe().expect("integration test executable"))
        .arg("--exact")
        .arg("rn4e_child_worker")
        .arg("--nocapture")
        .env("RN4E_CHILD_MODE", mode)
        .env("RN4E_APP", app)
        .env("RN4E_ROOT", root)
        .env("RN4E_DB", db)
        .env("RN4E_VAULT", vault.to_string())
        .env("RN4E_DEVICE", device.to_string())
        .env("RN4E_RESULT", result_file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start separate OS process")
}

fn wait_for_child_result(path: &Path) -> String {
    let start = Instant::now();
    loop {
        if let Ok(value) = fs::read_to_string(path) {
            if !value.is_empty() {
                return value;
            }
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "second process did not produce its independent result"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

/// Test-only fixture, NOT a Core API. It is intentionally incapable of
/// accepting a remote manifest path or arbitrary staged object. Production
/// needs a verified pinned stage, Core-owned path authority and a durable
/// pre-publication intent before this kind of guarded call is allowed.
fn fixture_publish_inside_fence(
    ledger: &mut RegisteredPullGrantLedger<'_>,
    core: &Core,
    expected: StoredPull,
    actual_remote: [u8; 32],
    now: u64,
    destination: &Path,
    content: &str,
) -> Result<(), GrantLedgerError> {
    if content.len() as u64 != expected.object_length
        || *blake3::hash(content.as_bytes()).as_bytes() != expected.object_blake3
    {
        return Err(GrantLedgerError::ScopeChanged);
    }
    ledger.with_fenced_pull_preflight(expected, actual_remote, now, |_| {
        // The real Core File Gate stays INSIDE BEGIN IMMEDIATE, unlike a
        // post-return write. A successful gate result is NOT durable APPLIED.
        gate::create(core, destination, content, Source::Me, None)
            .expect("the disposable File Gate fixture must publish");
        Ok(())
    })
}

/// Spawned by the actual Rust test executable with --exact. No shell and no
/// process-local mock Mutex. Every child has a separate SQLite connection.
/// This helper exits immediately in an ordinary parent test run.
#[test]
fn rn4e_child_worker() {
    let Ok(mode) = std::env::var("RN4E_CHILD_MODE") else {
        return;
    };
    let app = PathBuf::from(std::env::var_os("RN4E_APP").unwrap());
    let root = PathBuf::from(std::env::var_os("RN4E_ROOT").unwrap());
    let db = PathBuf::from(std::env::var_os("RN4E_DB").unwrap());
    let result_path = PathBuf::from(std::env::var_os("RN4E_RESULT").unwrap());
    let vault = VaultId::from_str(&std::env::var("RN4E_VAULT").unwrap()).unwrap();
    let device = DeviceId::from_str(&std::env::var("RN4E_DEVICE").unwrap()).unwrap();

    let outcome = match mode.as_str() {
        "zero_timeout_writer" => {
            let connection = Connection::open(&db).unwrap();
            connection.busy_timeout(Duration::ZERO).unwrap();
            match connection.execute(
                "UPDATE pull_grants SET epoch=2,active=0
                 WHERE vault=?1 AND device=?2 AND epoch=1 AND active=1",
                params![vault.to_string(), device.to_string()],
            ) {
                Err(rusqlite::Error::SqliteFailure(e, _)) if e.code == ErrorCode::DatabaseBusy => {
                    "busy"
                }
                Ok(1) => "revoked",
                other => panic!("unexpected competing SQLite outcome: {other:?}"),
            }
        }
        "core_revoke_first" => {
            let core = registered_core(&app, &root);
            let mut ledger = RegisteredPullGrantLedger::open_existing(&core, &root, &db).unwrap();
            assert_eq!(ledger.revoke(vault, device, 1).unwrap(), 2);
            "revoked"
        }
        other => panic!("unexpected child test mode: {other}"),
    };
    fs::write(result_path, outcome).unwrap();
}

#[test]
fn cross_process_writer_cannot_revoke_during_real_filegate_create() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("appdata");
    let root = tmp.path().join("vault");
    let db = app.join("grants.sqlite3");
    let result = app.join("child-writer-outcome");
    let target = root.join("RN4E0.md");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let device = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let expected = ledger.issue(proposal(vault, device)).unwrap();
    let mut competitor = None;

    ledger
        .with_fenced_pull_preflight(expected, [7; 32], NOW, |_| {
            competitor = Some(spawn_competing_process(
                "zero_timeout_writer",
                &app,
                &root,
                &db,
                vault,
                device,
                &result,
            ));
            // Require proof that a *different process* has attempted the
            // conflicting SQLite write before entering the real File Gate.
            assert_eq!(wait_for_child_result(&result), "busy");
            let created = gate::create(&core, &target, CONTENT, Source::Me, None)
                .expect("the existing Core File Gate must create new local note");
            assert_eq!(
                created.hash,
                blake3::hash(CONTENT.as_bytes()).to_hex().to_string()
            );
            assert_eq!(fs::read(&target).unwrap(), CONTENT.as_bytes());
            Ok(())
        })
        .unwrap();
    assert!(competitor.take().unwrap().wait().unwrap().success());
    assert_eq!(ledger.read_active(vault, device, NOW).unwrap(), expected);
    assert_eq!(ledger.revoke(vault, device, 1).unwrap(), 2);
    assert_eq!(fs::read(&target).unwrap(), CONTENT.as_bytes());
    assert!(matches!(
        ledger.with_fenced_pull_preflight(expected, [7; 32], NOW, |_| {
            panic!("revoked grant must never enter the writer");
        }),
        Err(GrantLedgerError::Revoked)
    ));
    // Deliberately no APPLIED, no signed receipt, and no remote ACK.
}

#[test]
fn revoke_committed_in_another_core_process_denies_filegate_before_write() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("appdata");
    let root = tmp.path().join("vault");
    let db = app.join("grants.sqlite3");
    let result = app.join("child-revoke-outcome");
    let target = root.join("no-revoked-publish.md");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let device = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let snapshot = ledger.issue(proposal(vault, device)).unwrap();

    let mut competitor = spawn_competing_process(
        "core_revoke_first",
        &app,
        &root,
        &db,
        vault,
        device,
        &result,
    );
    assert_eq!(wait_for_child_result(&result), "revoked");
    assert!(competitor.wait().unwrap().success());

    assert!(matches!(
        fixture_publish_inside_fence(&mut ledger, &core, snapshot, [7; 32], NOW, &target, CONTENT,),
        Err(GrantLedgerError::Revoked)
    ));
    assert!(!target.exists());
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}

#[test]
fn exact_object_and_endpoint_must_match_before_disposable_publication() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("appdata");
    let root = tmp.path().join("vault");
    let db = app.join("grants.sqlite3");
    let target = root.join("no-corrupt-publish.md");
    let core = registered_core(&app, &root);
    let vault = VaultId::new();
    let device = DeviceId::new();
    let mut ledger = RegisteredPullGrantLedger::provision_new(&core, &root, &db).unwrap();
    let expected = ledger.issue(proposal(vault, device)).unwrap();

    for invalid in [
        "tampered payload",
        "RN-4E0 verified local fixture - not network payloaD\n",
    ] {
        assert!(matches!(
            fixture_publish_inside_fence(
                &mut ledger,
                &core,
                expected,
                [7; 32],
                NOW,
                &target,
                invalid,
            ),
            Err(GrantLedgerError::ScopeChanged)
        ));
    }
    assert!(matches!(
        fixture_publish_inside_fence(
            &mut ledger,
            &core,
            expected,
            [99; 32],
            NOW,
            &target,
            CONTENT,
        ),
        Err(GrantLedgerError::WrongAuthenticatedEndpoint)
    ));
    assert!(matches!(
        fixture_publish_inside_fence(
            &mut ledger,
            &core,
            expected,
            [7; 32],
            expected.expires_at,
            &target,
            CONTENT,
        ),
        Err(GrantLedgerError::InvalidProposal)
    ));
    assert!(!target.exists());
    assert!(fs::read_dir(&root).unwrap().next().is_none());
}
