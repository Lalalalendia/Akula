//! RN-4E1b-03c1b1a: independent monotonic witness contract, test-only.
//!
//! A correctly checksummed journal alone CANNOT prove that an old valid copy
//! has not replaced current deny-floor state. Verification requires trusted
//! EXTERNAL installer/TPM-owned monotonically sealed facts. Here, these are
//! TEST FIXTURES, NOT actual persisted/attested credentials.
//!
//! No runtime witness constructor, no Windows service integration, no permit,
//! no IPC, no QUIC, no file writer. This module is compiled only for Windows
//! Rust unit tests, never into Akula's installed application.
//!
//! Read-check holds a single kernel-exclusive journal handle through complete
//! validation and witness comparison. If journal and trusted witness disagree
//! in EITHER direction (write-before-seal / seal-before-write), outcome HOLD
//! and a privileged recovery process must reconcile; never auto-advance trust.

#![cfg(all(test, windows))]

use super::windows_revocation_journal::{exclusive_existing, read_log, Hold as JournalHold};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Hold {
    NoExternalWitness,
    WrongInstallerBinding,
    Journal(JournalHold),
    // A valid old snapshot OR an interrupted commit can leave the file
    // behind the external witness. We cannot distinguish without recovery.
    JournalBehindWitness,
    // Journal fsync may have succeeded, but independent trust never sealed
    // it. The new record is not permission and cannot self-promote.
    JournalAheadOfWitness,
    DivergentFloorMap,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
enum Outcome {
    Hold(Hold),
    /// Never an Allow. Even trusted deny floors prove NOTHING about an
    /// authorized QUIC peer or File Gate write capability.
    ConsistentDenialSnapshotOnly,
}

/// Merely TEST-SUPPLIED externally sealed facts. There is no runtime source
/// and no way to construct or persist this witness outside the test build.
/// A production contract needs a canonical owner, physical installer anchor,
/// non-rollbackable monotonic storage, and verified signature/hardware state.
#[derive(Clone, Debug)]
struct ExternalWitnessFixture {
    installer_binding: [u8; 16],
    expected_sequence: u64,
    expected_floors: BTreeMap<[u8; 16], u64>,
}

/// The ONLY reader of the journal is invoked with external expected state.
/// It NEVER treats whatever bytes it sees as its own source of truth.
fn inspect_against_external_witness(
    journal: &Path,
    expected_binding: [u8; 16],
    witness: Option<&ExternalWitnessFixture>,
) -> Outcome {
    let Some(expected) = witness else {
        return Outcome::Hold(Hold::NoExternalWitness);
    };
    if expected.installer_binding != expected_binding || expected_binding == [0; 16] {
        return Outcome::Hold(Hold::WrongInstallerBinding);
    }
    let mut file = match exclusive_existing(journal) {
        Ok(file) => file,
        Err(error) => return Outcome::Hold(Hold::Journal(error)),
    };
    let actual = match read_log(&mut file, expected_binding) {
        Ok(log) => log,
        Err(error) => return Outcome::Hold(Hold::Journal(error)),
    };
    if actual.sequence < expected.expected_sequence {
        return Outcome::Hold(Hold::JournalBehindWitness);
    }
    if actual.sequence > expected.expected_sequence {
        return Outcome::Hold(Hold::JournalAheadOfWitness);
    }
    if actual.floors != expected.expected_floors {
        return Outcome::Hold(Hold::DivergentFloorMap);
    }
    Outcome::ConsistentDenialSnapshotOnly
}

#[cfg(test)]
mod tests {
    use super::super::windows_revocation_journal::revoke_existing;
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const BINDING: [u8; 16] = [13; 16];
    const DEVICE_A: [u8; 16] = [7; 16];
    const DEVICE_B: [u8; 16] = [8; 16];

    struct JournalFixture {
        directory: PathBuf,
        path: PathBuf,
    }

    impl JournalFixture {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "akula-witness-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&directory).unwrap();
            let path = directory.join("preinstalled-deny-only.journal");
            let mut handle = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            // Independently preprovisioned journal header, NOT a grant DB.
            let mut header = [0_u8; 32];
            header[..8].copy_from_slice(b"AKRVC01\0");
            header[8..12].copy_from_slice(&1u32.to_le_bytes());
            header[12..28].copy_from_slice(&BINDING);
            handle.write_all(&header).unwrap();
            handle.sync_all().unwrap();
            Self { directory, path }
        }
    }

    impl Drop for JournalFixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.directory).unwrap();
        }
    }

    fn witness(sequence: u64, entries: &[([u8; 16], u64)]) -> ExternalWitnessFixture {
        ExternalWitnessFixture {
            installer_binding: BINDING,
            expected_sequence: sequence,
            expected_floors: entries.iter().copied().collect(),
        }
    }

    fn inspect(journal: &Path, expected: Option<&ExternalWitnessFixture>) -> Outcome {
        inspect_against_external_witness(journal, BINDING, expected)
    }

    #[test]
    fn unprovisioned_witness_is_never_inferred_from_perfectly_valid_journal() {
        let j = JournalFixture::new("no-witness");
        assert_eq!(inspect(&j.path, None), Outcome::Hold(Hold::NoExternalWitness));
        assert_eq!(revoke_existing(&j.path, BINDING, DEVICE_A, 0), Ok(1));
        assert_eq!(inspect(&j.path, None), Outcome::Hold(Hold::NoExternalWitness));
    }

    #[test]
    fn matching_external_snapshot_proves_only_consistency_not_authorization() {
        let j = JournalFixture::new("matching");
        let w0 = witness(0, &[]);
        assert_eq!(inspect(&j.path, Some(&w0)), Outcome::ConsistentDenialSnapshotOnly);
        assert_eq!(revoke_existing(&j.path, BINDING, DEVICE_A, 0), Ok(1));
        let w1 = witness(1, &[(DEVICE_A, 1)]);
        assert_eq!(inspect(&j.path, Some(&w1)), Outcome::ConsistentDenialSnapshotOnly);
        // No permit or positive P2P policy operation exists in this API.
    }

    #[test]
    fn previous_valid_journal_snapshot_fails_when_external_witness_is_newer() {
        let j = JournalFixture::new("rollback");
        let valid_old = fs::read(&j.path).unwrap();
        assert_eq!(revoke_existing(&j.path, BINDING, DEVICE_A, 0), Ok(1));
        let w1 = witness(1, &[(DEVICE_A, 1)]);
        assert_eq!(inspect(&j.path, Some(&w1)), Outcome::ConsistentDenialSnapshotOnly);
        // The adversary can replace the whole on-disk file while no reader
        // has it open; its CRC/sequence/header remain perfectly valid.
        fs::write(&j.path, valid_old).unwrap();
        assert_eq!(inspect(&j.path, Some(&w1)), Outcome::Hold(Hold::JournalBehindWitness));
    }

    #[test]
    fn unsealed_journal_advance_and_sealed_but_missing_write_both_hold() {
        let j = JournalFixture::new("two-stage");
        let w0 = witness(0, &[]);
        let w1 = witness(1, &[(DEVICE_A, 1)]);
        assert_eq!(inspect(&j.path, Some(&w1)), Outcome::Hold(Hold::JournalBehindWitness));
        assert_eq!(revoke_existing(&j.path, BINDING, DEVICE_A, 0), Ok(1));
        assert_eq!(inspect(&j.path, Some(&w0)), Outcome::Hold(Hold::JournalAheadOfWitness));
        assert_eq!(inspect(&j.path, Some(&w1)), Outcome::ConsistentDenialSnapshotOnly);
    }

    #[test]
    fn same_sequence_with_other_replica_floor_is_not_equivalent() {
        let j = JournalFixture::new("floor-map");
        assert_eq!(revoke_existing(&j.path, BINDING, DEVICE_B, 0), Ok(1));
        let other_replica_witness = witness(1, &[(DEVICE_A, 1)]);
        assert_eq!(
            inspect(&j.path, Some(&other_replica_witness)),
            Outcome::Hold(Hold::DivergentFloorMap)
        );
        let matching = witness(1, &[(DEVICE_B, 1)]);
        assert_eq!(inspect(&j.path, Some(&matching)), Outcome::ConsistentDenialSnapshotOnly);
    }

    #[test]
    fn incompatible_installer_binding_or_corruption_cannot_bootstrap() {
        let j = JournalFixture::new("binding");
        let w0 = witness(0, &[]);
        let mut swapped = w0.clone();
        swapped.installer_binding = [99; 16];
        assert_eq!(
            inspect(&j.path, Some(&swapped)),
            Outcome::Hold(Hold::WrongInstallerBinding)
        );
        let missing = j.directory.join("missing");
        assert!(matches!(
            inspect(&missing, Some(&w0)),
            Outcome::Hold(Hold::Journal(JournalHold::MissingOrUnreadable))
        ));
        let mut append = OpenOptions::new().append(true).open(&j.path).unwrap();
        append.write_all(&[0x99]).unwrap();
        append.sync_all().unwrap();
        drop(append);
        assert_eq!(
            inspect(&j.path, Some(&w0)),
            Outcome::Hold(Hold::Journal(JournalHold::CorruptOrTorn))
        );
    }

    #[test]
    fn any_unknown_earlier_or_later_sequence_stays_hold_not_auto_sync() {
        let j = JournalFixture::new("unknown-sequence");
        let high = witness(u64::MAX, &[]);
        assert_eq!(inspect(&j.path, Some(&high)), Outcome::Hold(Hold::JournalBehindWitness));
        assert_eq!(revoke_existing(&j.path, BINDING, DEVICE_A, 0), Ok(1));
        let empty = witness(0, &[]);
        assert_eq!(inspect(&j.path, Some(&empty)), Outcome::Hold(Hold::JournalAheadOfWitness));
    }
}
