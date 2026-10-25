//! Process-kill experiment. A separate Rust test process writes an fsynced
//! checkpoint, announces readiness and is forcibly terminated by its parent.
//! The parent cold-reopens the stage and checks that the partial transfer was
//! NOT published as a complete Inbox entry. This is not power-loss testing.
//! The stage/sidecar design is not Windows-ACL hardened or atomic across files.
use super::*;
use super::resume_checkpoint_lab::{self, Identity};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const FULL_SIZE: usize = 5 * 1024 * 1024;
const FIRST_SIZE: usize = 2 * 1024 * 1024;

fn identity() -> Identity {
    Identity {
        sender: [0x18; 32],
        receiver: [0x29; 32],
        full_hash: *blake3::hash(&vec![0x73; FULL_SIZE]).as_bytes(),
        total: FULL_SIZE as u64,
    }
}

/// Invoked only from a new OS process by the parent test. This child can
/// neither acknowledge the whole file nor make it visible in the Inbox.
#[test]
#[ignore = "run only as a disposable child via force_kill_after_checkpoint"]
fn writer_child_pauses_after_fsynced_checkpoint() {
    let root = std::env::var_os("AKULA_PROCESS_KILL_ROOT")
        .expect("only the parent should execute the ignored child");
    let root = Path::new(&root);
    let checkpoint = resume_checkpoint_lab::persist(
        &root.join("partial-input.bin"),
        &root.join("private-partial.stage"),
        &root.join("private-partial.checkpoint"),
        identity(),
        FIRST_SIZE as u64,
    )
    .expect("durable stage and sidecar created in this test fixture");
    assert_eq!(checkpoint.offset, FIRST_SIZE as u64);
    let mut ready = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(root.join("ready-to-kill.marker"))
        .unwrap();
    ready.write_all(b"CHECKPOINT_FSYNC_COMPLETE").unwrap();
    ready.sync_all().unwrap();
    drop(ready);
    thread::sleep(Duration::from_secs(60));
    panic!("child should have been killed before it could exit");
}

#[test]
fn force_kill_after_checkpoint_cold_recover_and_complete_without_false_ack() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let expected = identity();
    let full = vec![0x73; FULL_SIZE];
    fs::write(root.join("partial-input.bin"), &full[..FIRST_SIZE]).unwrap();
    let inbox_root = root.join("inbox");
    fs::create_dir(&inbox_root).unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "resume_process_lab::writer_child_pauses_after_fsynced_checkpoint",
            "--ignored",
            "--nocapture",
        ])
        .env("AKULA_PROCESS_KILL_ROOT", root)
        .stdout(Stdio::null())
        .spawn()
        .expect("start separate Rust process to simulate interruption");

    let ready = root.join("ready-to-kill.marker");
    let mut signalled = false;
    for _ in 0..400 {
        if ready.exists() {
            signalled = true;
            break;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("child exited before successful checkpoint: {status}");
        }
        thread::sleep(Duration::from_millis(25));
    }
    if !signalled {
        child.kill().unwrap();
        let _ = child.wait();
        panic!("child did not fsync its private checkpoint before deadline");
    }

    // Actual OS process termination: no Drop, graceful stream shutdown or
    // application-completion path can run in the child after this point.
    child.kill().expect("force-kill checkpoint writer");
    let status = child.wait().unwrap();
    assert!(!status.success());

    let cp = resume_checkpoint_lab::recover(
        &root.join("private-partial.stage"),
        &root.join("private-partial.checkpoint"),
        expected,
    )
    .expect("cold physical file and independent BLAKE3 prefix validated");
    assert_eq!(cp.offset, FIRST_SIZE as u64);
    let ledger = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
    assert_eq!(ledger.count().unwrap(), 0);
    drop(ledger);

    // Parent mimics the bytes of the resumed network suffix; the real QUIC
    // reconnection is covered by resume_lab, not by this process-kill test.
    let mut stage = OpenOptions::new()
        .append(true)
        .open(root.join("private-partial.stage"))
        .unwrap();
    stage.write_all(&full[FIRST_SIZE..]).unwrap();
    stage.sync_all().unwrap();
    drop(stage);
    let mut stage = File::open(root.join("private-partial.stage")).unwrap();
    let mut complete = Vec::new();
    use std::io::Read;
    stage.read_to_end(&mut complete).unwrap();
    assert_eq!(complete.len(), FULL_SIZE);
    assert_eq!(*blake3::hash(&complete).as_bytes(), expected.full_hash);
    let saved = inbox::persist_received_payload(
        &complete,
        &inbox_root,
        FULL_SIZE as u64,
        expected.full_hash,
        expected.sender,
        expected.receiver,
    )
    .expect("only a fully verified 5-MiB photo enters private Inbox");
    let cold = akula_inbox_ledger_lab::InboxLedger::open(&inbox_root).unwrap();
    assert_eq!(cold.count().unwrap(), 1);
    assert_eq!(fs::read(cold.object_path(saved.id)).unwrap(), full);
}
