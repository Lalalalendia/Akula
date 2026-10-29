//! NODE-E2E-01a: real Linux subprocess evidence for the existing Sync DAG
//! blob writer used as an operator-only, single-document Node replica.
//!
//! The subprocess helper is only an integration-test entrypoint. It is not
//! a CLI command, a network endpoint, a pairing protocol or user approval.
#![cfg(all(target_os = "linux", feature = "node-replica-lab"))]

use aquilum_core::node_replica_lab::{
    OperatorFixtureAdmission, stage_operator_fixture_with_hook, verify_local_replica,
};
use aquilum_core::sync::{DocumentId, RevisionId, VaultId};
use rusqlite::params;
use std::env;
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn vault() -> VaultId { VaultId::from_uuid(uuid::Uuid::from_u128(0x1111)) }
fn document() -> DocumentId { DocumentId::from_uuid(uuid::Uuid::from_u128(0x2222)) }
fn revision() -> RevisionId { RevisionId::from_uuid(uuid::Uuid::from_u128(0x3333)) }

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temporary = tempfile::tempdir().unwrap();
    let source = temporary.path().join("vault");
    let state = temporary.path().join("node-state");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&state).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    (temporary, source, state)
}

fn admission(bytes: &[u8]) -> OperatorFixtureAdmission {
    OperatorFixtureAdmission {
        vault_id: vault(),
        document_id: document(),
        revision_id: revision(),
        filename: "public.md".into(),
        expected_blake3: blake3::hash(bytes).to_hex().to_string(),
    }
}

fn db_path(state: &Path, v: VaultId) -> PathBuf {
    state.join(format!("akula-node-replica-{v}.sqlite"))
}

fn child(mode: &str, root: &Path, state: &Path, a: &OperatorFixtureAdmission, stop: bool)
    -> Output
{
    Command::new(env::current_exe().unwrap())
        .arg("--exact").arg("replica_child_entry").arg("--nocapture")
        .env("AKULA_NODE_REPLICA_CHILD", mode)
        .env("AKULA_NODE_REPLICA_ROOT", root)
        .env("AKULA_NODE_REPLICA_STATE", state)
        .env("AKULA_NODE_REPLICA_VAULT", a.vault_id.to_string())
        .env("AKULA_NODE_REPLICA_DOCUMENT", a.document_id.to_string())
        .env("AKULA_NODE_REPLICA_REVISION", a.revision_id.to_string())
        .env("AKULA_NODE_REPLICA_FILENAME", &a.filename)
        .env("AKULA_NODE_REPLICA_HASH", &a.expected_blake3)
        .env("AKULA_NODE_REPLICA_STOP", if stop { "yes" } else { "no" })
        .output().unwrap()
}

fn success(output: &Output) {
    assert!(output.status.success(), "child failed: {}",
        String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("NODE_REPLICA_LAB_VERIFIED="),
        "missing independent readback witness");
}

#[test]
fn replica_child_entry() {
    let Ok(mode) = env::var("AKULA_NODE_REPLICA_CHILD") else { return };
    let root = PathBuf::from(env::var_os("AKULA_NODE_REPLICA_ROOT").unwrap());
    let state = PathBuf::from(env::var_os("AKULA_NODE_REPLICA_STATE").unwrap());
    let a = OperatorFixtureAdmission {
        vault_id: env::var("AKULA_NODE_REPLICA_VAULT").unwrap().parse().unwrap(),
        document_id: env::var("AKULA_NODE_REPLICA_DOCUMENT").unwrap().parse().unwrap(),
        revision_id: env::var("AKULA_NODE_REPLICA_REVISION").unwrap().parse().unwrap(),
        filename: env::var("AKULA_NODE_REPLICA_FILENAME").unwrap(),
        expected_blake3: env::var("AKULA_NODE_REPLICA_HASH").unwrap(),
    };
    let bytes = match mode.as_ref() {
        "stage" => {
            let proof = stage_operator_fixture_with_hook(&root, &state, &a, || {
                if env::var("AKULA_NODE_REPLICA_STOP").ok().as_deref() == Some("yes") {
                    // Abrupt exit after a durable blob but before its DAG record.
                    std::process::exit(73);
                }
            }).expect("fixture stage must satisfy all guards");
            assert_eq!(proof.raw_blake3, a.expected_blake3);
            verify_local_replica(&state, &a).expect("stage requires readback")
        }
        "verify" => verify_local_replica(&state, &a).expect("readback must pass"),
        _ => panic!("unknown integration-only child mode"),
    };
    assert_eq!(blake3::hash(&bytes).to_hex().to_string(), a.expected_blake3);
    println!("NODE_REPLICA_LAB_VERIFIED={} bytes={}", a.expected_blake3, bytes.len());
}

#[test]
fn two_real_processes_read_durable_replica_after_original_disappears() {
    let (_tmp, source, state) = fixture();
    let bytes = b"# local-first\r\nPrivate fixture in a separate Node store.\r\n";
    fs::write(source.join("public.md"), bytes).unwrap();
    let a = admission(bytes);
    success(&child("stage", &source, &state, &a, false));
    let metadata = fs::metadata(db_path(&state, vault())).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    fs::rename(source.join("public.md"), source.join("original-offline.md")).unwrap();

    // A separate OS process reconstructs the DAG + verifies the BLAKE3 blob
    // without any access to the original filename.
    success(&child("verify", &source, &state, &a, false));
    assert_eq!(fs::read(source.join("original-offline.md")).unwrap(), bytes);
    assert_eq!(fs::read_dir(&source).unwrap().count(), 1);
    println!("NODE_E2E_01A_TWO_PROCESS_LOCAL_REPLICA_PASS");
}

#[test]
fn denied_digest_wrong_vault_document_revision_and_unsafe_sources() {
    let (_tmp, source, state) = fixture();
    let bytes = b"operator admitted test note";
    fs::write(source.join("public.md"), bytes).unwrap();
    let a = admission(bytes);
    let mut wrong_hash = a.clone();
    wrong_hash.expected_blake3 = "0".repeat(64);
    assert!(!child("stage", &source, &state, &wrong_hash, false).status.success());
    assert!(!db_path(&state, vault()).exists(), "bad hash must fail before SQLite");
    success(&child("stage", &source, &state, &a, false));

    let mut wrong_vault = a.clone();
    wrong_vault.vault_id = VaultId::from_uuid(uuid::Uuid::from_u128(0x4444));
    let mut wrong_doc = a.clone();
    wrong_doc.document_id = DocumentId::from_uuid(uuid::Uuid::from_u128(0x5555));
    let mut wrong_revision = a.clone();
    wrong_revision.revision_id = RevisionId::from_uuid(uuid::Uuid::from_u128(0x6666));
    for mismatch in [&wrong_vault, &wrong_doc, &wrong_revision, &wrong_hash] {
        assert!(!child("verify", &source, &state, mismatch, false).status.success());
    }
    assert!(!db_path(&state, wrong_vault.vault_id).exists(),
        "unknown vault must not cause an empty DB");
    symlink(source.join("public.md"), source.join("alias.md")).unwrap();
    let mut alias = a.clone();
    alias.filename = "alias.md".into();
    assert!(!child("stage", &source, &state, &alias, false).status.success());
    alias.filename = "../outside.md".into();
    assert!(!child("stage", &source, &state, &alias, false).status.success());
    success(&child("verify", &source, &state, &a, false));
}

#[test]
fn abrupt_exit_before_revision_keeps_blob_unservable_until_explicit_retry() {
    let (_tmp, source, state) = fixture();
    let bytes = b"power-loss boundary before publishing revision";
    fs::write(source.join("public.md"), bytes).unwrap();
    let a = admission(bytes);
    let stopped = child("stage", &source, &state, &a, true);
    assert!(!stopped.status.success(), "force-stop must actually exit unsuccessfully");
    assert_eq!(stopped.status.code(), Some(73), "expected deterministic exit stage");
    assert!(db_path(&state, vault()).exists(), "blob stage genuinely committed");
    assert!(!child("verify", &source, &state, &a, false).status.success(),
        "orphan blob must never become a published revision");
    success(&child("stage", &source, &state, &a, false));
    success(&child("verify", &source, &state, &a, false));
}

#[test]
fn revision_collision_and_corrupted_sqlite_blob_never_serve_wrong_bytes() {
    let (_tmp, source, state) = fixture();
    let original = b"immutable revision original";
    fs::write(source.join("public.md"), original).unwrap();
    let a = admission(original);
    success(&child("stage", &source, &state, &a, false));

    fs::write(source.join("public.md"), b"changed without a new revision").unwrap();
    let mut collision = a.clone();
    collision.expected_blake3 = blake3::hash(b"changed without a new revision")
        .to_hex().to_string();
    assert!(!child("stage", &source, &state, &collision, false).status.success());
    success(&child("verify", &source, &state, &a, false));

    let db = db_path(&state, vault());
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection.execute(
        "UPDATE sync_blobs SET data = ?1 WHERE hash = ?2",
        params![b"corruption".as_slice(), &a.expected_blake3],
    ).unwrap();
    drop(connection);
    assert!(!child("verify", &source, &state, &a, false).status.success(),
        "checksum mismatch must be refused after process restart");
}

#[test]
fn private_state_must_not_be_inside_the_source_vault() {
    let (_tmp, source, _state) = fixture();
    let nested = source.join("forbidden-node-state");
    fs::create_dir(&nested).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    let bytes = b"do not initialize Node inside owner vault";
    fs::write(source.join("public.md"), bytes).unwrap();
    let a = admission(bytes);
    assert!(!child("stage", &source, &nested, &a, false).status.success());
    assert_eq!(fs::read_dir(&nested).unwrap().count(), 0);
}
