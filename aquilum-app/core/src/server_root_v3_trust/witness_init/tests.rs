use super::*;
use crate::server_witness_lab::{InertWitnessLedgerLab, WitnessCheckpoint};
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt};
use tempfile::tempdir;

fn lib_dir(parent: &Path) -> File {
    let lib = parent.join("lib");
    fs::create_dir(&lib).unwrap();
    fs::set_permissions(&lib, fs::Permissions::from_mode(0o755)).unwrap();
    File::open(lib).unwrap()
}
fn witness_files(base: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let p = base.join("lib/akula/server-bootstrap");
    (p.join("trust"), p.join("journal"))
}

#[test]
fn initial_witness_requires_preexisting_root_authority() {
    let tmp = tempdir().unwrap();
    let lib = lib_dir(tmp.path());
    if rustix::process::geteuid().as_raw() != 0 {
        assert!(matches!(
            initialize_once(&lib, Stop::Never),
            Err(WitnessGenesisError::NotRoot),
        ));
        assert!(!tmp.path().join("lib/akula").exists());
    }
}

#[test]
fn fresh_uid0_witness_genesis_has_random_key_and_empty_cas() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let tmp = tempdir().unwrap();
    let lib = lib_dir(tmp.path());
    initialize_once(&lib, Stop::Never).unwrap();

    let (trust, journal) = witness_files(tmp.path());
    for directory in [
        tmp.path().join("lib/akula"),
        tmp.path().join("lib/akula/server-bootstrap"),
        trust.clone(),
        journal.clone(),
    ] {
        let m = fs::metadata(directory).unwrap();
        assert!(root_dir_ok(&m, 0o700));
    }
    let seed = fs::read(trust.join(SEED)).unwrap();
    let pubkey = fs::read(trust.join(PUB)).unwrap();
    assert_eq!(seed.len(), 32);
    assert_ne!(seed, vec![0; 32]);
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
    assert_eq!(pubkey.as_slice(), pair.public_key().as_ref());
    assert_eq!(fs::read(trust.join(MARKER)).unwrap(), WITNESS_ENROLLMENT);
    for name in [SEED, PUB, MARKER] {
        assert!(root_file_ok(
            &fs::metadata(trust.join(name)).unwrap(),
            0o600,
            128
        ));
        assert!(!trust.join(format!("{name}.pending")).exists());
    }
    let journal_actor = InertWitnessLedgerLab::open(&journal).unwrap();
    assert_eq!(
        journal_actor.current_checkpoint().unwrap(),
        WitnessCheckpoint::genesis()
    );

    assert!(matches!(
        initialize_once(&lib, Stop::Never),
        Err(WitnessGenesisError::ExistingOrInterrupted),
    ));
    assert_eq!(fs::read(trust.join(SEED)).unwrap(), seed);
    assert_eq!(fs::read(trust.join(PUB)).unwrap(), pubkey);
}

#[test]
fn interrupted_root_genesis_is_hard_hold_with_no_rerun_or_marker() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    for stop in [Stop::AfterDirectories, Stop::AfterSeed, Stop::AfterPublic] {
        let tmp = tempdir().unwrap();
        let lib = lib_dir(tmp.path());
        assert!(matches!(
            initialize_once(&lib, stop),
            Err(WitnessGenesisError::ExistingOrInterrupted),
        ));
        let (trust, journal) = witness_files(tmp.path());
        assert!(trust.exists());
        assert!(journal.exists());
        assert!(!trust.join(MARKER).exists());
        assert!(matches!(
            initialize_once(&lib, Stop::Never),
            Err(WitnessGenesisError::ExistingOrInterrupted),
        ));
        assert!(!trust.join(MARKER).exists());
    }
}

#[test]
fn foreign_directory_symlink_and_nonroot_owned_parent_refuse_without_writes() {
    if rustix::process::geteuid().as_raw() != 0 {
        return;
    }
    let tmp = tempdir().unwrap();
    let lib = lib_dir(tmp.path());
    let destination = tmp.path().join("elsewhere");
    fs::create_dir(&destination).unwrap();
    symlink(&destination, tmp.path().join("lib/akula")).unwrap();
    assert!(matches!(
        initialize_once(&lib, Stop::Never),
        Err(WitnessGenesisError::ExistingOrInterrupted),
    ));
    assert!(fs::read_dir(destination).unwrap().next().is_none());

    let tmp = tempdir().unwrap();
    let lib = lib_dir(tmp.path());
    fs::set_permissions(tmp.path().join("lib"), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(matches!(
        initialize_once(&lib, Stop::Never),
        Err(WitnessGenesisError::UntrustedParent),
    ));
    assert!(!tmp.path().join("lib/akula").exists());
}
