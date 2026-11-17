//! VIDEO-18b-17 NON-RELEASE: real five PE bytes, protected Program Files
//! installation, the *existing* rar2 UpdateEngine and its OS installation lock.
//! Pasted into pinned rar2-orchestrator/tests by PR-only Windows CI.
//! Candidate PEs carry lab-only overlay markers to make old/new genuinely
//! byte-distinct; these are NOT publisher-authenticated signed releases.
#![cfg(windows)]
use chaptera_update_engine::{RecoveryOutcome, UpdateEngine, UpdatePhase};
use chaptera_update_orchestrator::{InstallLock, OrchestrationError};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const COMPONENTS: [&str; 5] = [
    "akula-app.exe",
    "shiva-pdf-preview.exe",
    "akula-video-mux.exe",
    "akula-yt-dlp.exe",
    "akula-quickjs.exe",
];
const LAB_PREFIX: &str = "AkulaVideo18bRealNsisLab-";

fn regular(path: &Path, directory: bool) {
    let info = fs::symlink_metadata(path).expect("VIDEO18B17_HOLD: expected path missing");
    assert!(
        !info.file_type().is_symlink()
            && if directory { info.file_type().is_dir() } else { info.file_type().is_file() },
        "VIDEO18B17_HOLD: wrong file type, symlink, or junction: {}",
        path.display()
    );
}

fn digest(path: &Path) -> String {
    regular(path, false);
    let mut input = File::open(path).unwrap();
    let size = input.metadata().unwrap().len();
    assert!((512..=512 * 1024 * 1024).contains(&size), "VIDEO18B17_HOLD: implausible PE length");
    let mut mz = [0u8; 2];
    input.read_exact(&mut mz).unwrap();
    assert_eq!(&mz, b"MZ", "VIDEO18B17_HOLD: input lost PE magic");
    let mut hash = Sha256::new();
    hash.update(mz);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut buf).unwrap();
        if count == 0 { break; }
        hash.update(&buf[..count]);
    }
    let output = hash.finalize();
    output.iter().map(|v| format!("{v:02x}")).collect()
}

fn hashes(path: &Path) -> BTreeMap<String, String> {
    regular(path, true);
    let entries = fs::read_dir(path).unwrap().collect::<std::io::Result<Vec<_>>>().unwrap();
    assert_eq!(entries.len(), COMPONENTS.len(), "VIDEO18B17_HOLD: unexpected generation members");
    let mut found = BTreeMap::new();
    for name in COMPONENTS {
        found.insert(name.to_owned(), digest(&path.join(name)));
    }
    found
}

fn setup() -> (PathBuf, PathBuf, BTreeMap<String, String>) {
    for (name, expected) in [
        ("GITHUB_ACTIONS", "true"),
        ("RUNNER_OS", "Windows"),
        ("RUNNER_ENVIRONMENT", "github-hosted"),
        ("GITHUB_EVENT_NAME", "pull_request"),
    ] {
        assert_eq!(
            env::var(name).unwrap_or_default(), expected,
            "VIDEO18B17_HOLD: source may run only in isolated hosted Windows PR"
        );
    }
    let pf = fs::canonicalize(env::var("ProgramFiles").unwrap()).unwrap();
    let root = fs::canonicalize(env::var("AKULA_VIDEO18B17_INSTALL_ROOT").unwrap()).unwrap();
    let leaf = root.file_name().and_then(|v| v.to_str()).unwrap();
    let Some(tag) = leaf.strip_prefix(LAB_PREFIX) else {
        panic!("VIDEO18B17_HOLD: install root not an Akula lab");
    };
    assert!(
        root.parent() == Some(pf.as_path())
            && tag.len() == 32
            && tag.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "VIDEO18B17_HOLD: cannot address any ordinary Program Files installation"
    );
    regular(&root, true);
    let temp = fs::canonicalize(env::var("RUNNER_TEMP").unwrap()).unwrap();
    let stage = fs::canonicalize(env::var("AKULA_VIDEO18B17_STAGE_ROOT").unwrap()).unwrap();
    assert!(
        stage.parent() == Some(temp.as_path())
            && stage.file_name().and_then(|s| s.to_str()).unwrap_or_default()
                == format!("Video18b16Stage-{tag}"),
        "VIDEO18B17_HOLD: candidate staging outside exact isolated run"
    );
    regular(&stage, true);
    let baseline = hashes(&root.join("current"));
    for (name, expected) in &baseline {
        assert_eq!(
            digest(&root.join(name)), *expected,
            "VIDEO18B17_HOLD: protected flat fallback differs from current"
        );
    }
    regular(&root.join("uninstall.exe"), false);
    assert_eq!(
        fs::read_dir(&root).unwrap().count(), 7,
        "VIDEO18B17_HOLD: protected root contains unexpected entries"
    );
    (root, stage, baseline)
}

fn stage_candidate(stage: &Path, original: &BTreeMap<String, String>, root: &Path) -> PathBuf {
    let candidate = stage.join("video18b17-lab-only-candidate");
    fs::create_dir(&candidate).expect("VIDEO18B17_HOLD: candidate stage reuse");
    for name in COMPONENTS {
        let source = root.join("current").join(name);
        let destination = candidate.join(name);
        fs::copy(source, &destination).unwrap();
        let mut out = OpenOptions::new().append(true).open(&destination).unwrap();
        // Distinct byte generation of genuine PE-shaped lab fixtures.
        // Invalidate any code signature on purpose. NEVER launch these files.
        out.write_all(format!("\nAKULA_VIDEO18B17_UNTRUSTED_LAB_OVERLAY::{name}\n").as_bytes()).unwrap();
        out.sync_all().unwrap();
        assert_ne!(digest(&destination), original[name]);
    }
    assert_eq!(hashes(&candidate).len(), 5);
    candidate
}

fn assert_old(root: &Path, old: &BTreeMap<String, String>) {
    assert_eq!(&hashes(&root.join("current")), old, "VIDEO18B17_HOLD: old generation mixed");
    for (name, hash) in old {
        assert_eq!(digest(&root.join(name)), *hash,
            "VIDEO18B17_HOLD: legacy entrypoints changed");
    }
}

fn assert_new(root: &Path, next: &BTreeMap<String, String>, old: &BTreeMap<String, String>) {
    assert_eq!(&hashes(&root.join("current")), next, "VIDEO18B17_HOLD: candidate not all five");
    for (name, hash) in old {
        assert_eq!(digest(&root.join(name)), *hash,
            "VIDEO18B17_HOLD: legacy entrypoints must remain old during staging");
    }
}

fn run_phase(
    root: &Path, candidate: &Path, old: &BTreeMap<String,String>,
    next: &BTreeMap<String,String>, suffix: &str,
) {
    let engine=UpdateEngine::new(root);
    assert!(engine.read_journal().unwrap().is_none());
    let tx=format!("video18b17-{suffix}");
    let copied_control=engine.begin_verified_candidate(
        &tx,"0.3.21",&candidate,Path::new("akula-app.exe")
    ).unwrap();
    assert_eq!(digest(&copied_control), old["akula-app.exe"],
        "VIDEO18B17_HOLD: control must be copied from OLD generation");
    assert_eq!(engine.read_journal().unwrap().unwrap().phase,UpdatePhase::Prepared);
    match suffix {
        "prepared" => {
            drop(engine);
            let fresh=UpdateEngine::new(root);
            assert_eq!(fresh.recover().unwrap(), RecoveryOutcome::PreparedTransactionAborted);
        }
        "retained" => {
            engine.retain_previous().unwrap();
            assert!(!root.join("current").exists());
            drop(engine);
            let fresh=UpdateEngine::new(root);
            assert_eq!(fresh.recover().unwrap(), RecoveryOutcome::UnconfirmedCandidateRolledBack);
        }
        "activated" => {
            engine.retain_previous().unwrap();
            engine.activate_candidate().unwrap();
            assert_new(root, next, old);
            drop(engine);
            let fresh=UpdateEngine::new(root);
            assert_eq!(fresh.recover().unwrap(), RecoveryOutcome::UnconfirmedCandidateRolledBack);
        }
        _ => panic!("VIDEO18B17_HOLD: unknown phase"),
    }
    let fresh=UpdateEngine::new(root);
    assert!(fresh.read_journal().unwrap().is_none(), "VIDEO18B17_HOLD: active transaction survived rollback");
    assert_old(root,old);
    fresh.cleanup_orphaned_transactions().unwrap();
}

#[test]
fn protected_real_five_pe_journal_and_os_lock_recovery() {
    let (root, stage, old) = setup();
    let candidate = stage_candidate(&stage,&old,&root);
    let next=hashes(&candidate);
    // InstallLock is the upstream OS-backed lock, not a stale PID-file convention.
    let owner=InstallLock::try_acquire(&root).unwrap();
    let loser=InstallLock::try_acquire(&root);
    assert!(
        matches!(loser,Err(OrchestrationError::LockBusy)),
        "VIDEO18B17_HOLD: a second owner acquired the protected install"
    );
    run_phase(&root,&candidate,&old,&next,"prepared");
    run_phase(&root,&candidate,&old,&next,"retained");
    run_phase(&root,&candidate,&old,&next,"activated");
    assert_old(&root,&old);
    drop(owner);
    // Demonstrate another genuine lock owner can proceed after first exit.
    let new_owner=InstallLock::try_acquire(&root).unwrap();
    drop(new_owner);

    let engine=UpdateEngine::new(&root);
    assert!(engine.read_journal().unwrap().is_none());
    engine.cleanup_orphaned_transactions().unwrap();
    for dirname in [".staging",".rollback"] {
        let location=root.join(dirname);
        if location.exists() {
            // Only removal of empty engine-created transaction directories.
            fs::remove_dir(location).unwrap();
        }
    }
    for filename in [
        "update-journal.json.prev", "update-journal.json.next", ".chaptera-install.lock"
    ] {
        let file=root.join(filename);
        if file.exists() { fs::remove_file(file).unwrap(); }
    }
    assert_eq!(fs::read_dir(&root).unwrap().count(),7,
        "VIDEO18B17_HOLD: engine left state that would break NSIS uninstall");
    assert_old(&root,&old);
    println!("VIDEO18B17_PROTECTED_REAL_PE_RAR2_RECOVERY_PASS cases=3 old_exes=5 candidate_exes=5 os_lock=1 flat_fallback_unchanged=1 uninstall_ready=1 release=0");
}
