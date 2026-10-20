//! RN-4E1b-03c1b0 — test-only, Windows OS-process durable DENY floor.
//!
//! This file is compiled ONLY under #[cfg(all(test, windows))]. No installed
//! Akula or actual SCM service can call it. This experiment deliberately
//! contains NO Enroll, Allow, File Gate, QUIC or positive peer permit.
//!
//! Exactly one pre-existing journal file is opened with Win32 share_mode(0)
//! (no other simultaneous reader/writer and no SHARE_DELETE). We parse the
//! ENTIRE append-only log under that OS lease before comparing the epoch.
//! A completed revoke is flushed to disk and read back under the same lease.
//! Failed writes, torn tails or unknown bytes are HOLD: no repair/truncate.
//!
//! CRC32 is ONLY accidental-corruption detection, NOT authentication. A
//! privileged adversary can forge/rollback this entire file; installer-owned
//! namespace, protected pinned ancestry, external rollback witness and
//! actual authenticated QUIC identity are future hard prerequisites.

#![cfg(all(test, windows))]

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::thread;
use std::time::Duration;

const MAGIC: [u8; 8] = *b"AKRVC01\0";
const VERSION: u32 = 1;
const HEADER: usize = 32;
const RECORD: usize = 64;
const MAX_RECORDS: usize = 4096;
const MAX_BYTES: usize = HEADER + MAX_RECORDS * RECORD;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const ERROR_SHARING_VIOLATION: i32 = 32;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum Hold {
    MissingOrUnreadable,
    Busy,
    CorruptOrTorn,
    WrongEpoch,
    EpochOverflow,
    SizeLimit,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) floors: BTreeMap<[u8; 16], u64>,
    pub(super) sequence: u64,
    previous_crc: u32,
}

fn crc32(data: &[u8]) -> u32 {
    let mut state = 0xffff_ffff_u32;
    for byte in data {
        state ^= u32::from(*byte);
        for _ in 0..8 {
            state = (state >> 1) ^ if state & 1 != 0 { 0xedb8_8320 } else { 0 };
        }
    }
    !state
}

fn little_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("slice sizes checked by parser"))
}
fn little_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes.try_into().expect("slice sizes checked by parser"))
}
fn invalid_io(_: io::Error) -> Hold {
    Hold::MissingOrUnreadable
}

/// Open an EXISTING installer-provisioned file, never CREATE. On Windows
/// share_mode(0) is a real kernel-enforced lifetime/identity fence, not a
/// process-local Mutex. Retries are bounded and preserve the expected epoch.
pub(super) fn exclusive_existing(path: &Path) -> Result<File, Hold> {
    for attempt in 0..50 {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(path)
        {
            Ok(file) => {
                let meta = file.metadata().map_err(invalid_io)?;
                if !meta.is_file()
                    || meta.file_attributes() & 0x0000_0400 != 0
                    || meta.len() > MAX_BYTES as u64
                {
                    // OS aliases and oversized journals never initialize
                    // a new empty epoch/permission universe.
                    return Err(Hold::SizeLimit);
                }
                return Ok(file);
            }
            Err(err) if err.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
                if attempt == 49 {
                    return Err(Hold::Busy);
                }
                thread::sleep(Duration::from_millis(8));
            }
            Err(_) => return Err(Hold::MissingOrUnreadable),
        }
    }
    Err(Hold::Busy)
}

/// Verify a full pre-existing log; no tail is skipped even if a valid old
/// prefix exists. Never convert a damaged tail into an empty/fresh registry.
pub(super) fn read_log(file: &mut File, expected_anchor: [u8; 16]) -> Result<State, Hold> {
    file.seek(SeekFrom::Start(0)).map_err(invalid_io)?;
    let mut bytes = Vec::new();
    (&mut *file).take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(invalid_io)?;
    if bytes.len() < HEADER || bytes.len() > MAX_BYTES {
        return Err(Hold::CorruptOrTorn);
    }
    if bytes[0..8] != MAGIC
        || little_u32(&bytes[8..12]) != VERSION
        || bytes[12..28] != expected_anchor
        || bytes[28..32] != [0u8; 4]
        || (bytes.len() - HEADER) % RECORD != 0
    {
        return Err(Hold::CorruptOrTorn);
    }
    let mut state = State::default();
    for record in bytes[HEADER..].chunks_exact(RECORD) {
        let next = state.sequence.checked_add(1).ok_or(Hold::CorruptOrTorn)?;
        let sequence = little_u64(&record[0..8]);
        let replica: [u8; 16] = record[8..24].try_into().unwrap();
        let expected_floor = little_u64(&record[24..32]);
        let new_floor = little_u64(&record[32..40]);
        let previous = little_u32(&record[40..44]);
        let checksum = little_u32(&record[60..64]);
        let floor = state.floors.get(&replica).copied().unwrap_or(0);
        if sequence != next
            || replica == [0; 16]
            || expected_floor != floor
            || new_floor != floor.checked_add(1).ok_or(Hold::CorruptOrTorn)?
            || previous != state.previous_crc
            || record[44] != 1
            || record[45..60] != [0u8; 15]
            || crc32(&record[0..60]) != checksum
        {
            return Err(Hold::CorruptOrTorn);
        }
        state.sequence = sequence;
        state.previous_crc = checksum;
        state.floors.insert(replica, new_floor);
    }
    Ok(state)
}

/// Full lifetime: OS-exclusive open -> whole-log verification -> exact CAS
/// -> append one denial floor -> sync_all -> full same-handle readback.
/// No transaction rollback is attempted after partial primary write.
pub(super) fn revoke_existing(
    path: &Path,
    anchor: [u8; 16],
    replica: [u8; 16],
    expected_floor: u64,
) -> Result<u64, Hold> {
    if replica == [0; 16] {
        return Err(Hold::CorruptOrTorn);
    }
    let mut file = exclusive_existing(path)?;
    let state = read_log(&mut file, anchor)?;
    if state.floors.get(&replica).copied().unwrap_or(0) != expected_floor {
        return Err(Hold::WrongEpoch);
    }
    if state.sequence >= MAX_RECORDS as u64 {
        return Err(Hold::SizeLimit);
    }
    let next_floor = expected_floor.checked_add(1).ok_or(Hold::EpochOverflow)?;
    let next_sequence = state.sequence.checked_add(1).ok_or(Hold::EpochOverflow)?;
    let mut row = [0u8; RECORD];
    row[0..8].copy_from_slice(&next_sequence.to_le_bytes());
    row[8..24].copy_from_slice(&replica);
    row[24..32].copy_from_slice(&expected_floor.to_le_bytes());
    row[32..40].copy_from_slice(&next_floor.to_le_bytes());
    row[40..44].copy_from_slice(&state.previous_crc.to_le_bytes());
    row[44] = 1; // DENY floor only. NO allow/enroll opcode.
    let crc = crc32(&row[0..60]);
    row[60..64].copy_from_slice(&crc.to_le_bytes());
    file.seek(SeekFrom::End(0)).map_err(invalid_io)?;
    file.write_all(&row).map_err(invalid_io)?;
    file.sync_all().map_err(invalid_io)?;
    let again = read_log(&mut file, anchor)?;
    if again.sequence != next_sequence
        || again.floors.get(&replica).copied() != Some(next_floor)
        || again.previous_crc != crc
    {
        return Err(Hold::CorruptOrTorn);
    }
    Ok(next_floor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    const ANCHOR: [u8; 16] = [13; 16];
    const REPLICA: [u8; 16] = [7; 16];

    struct TempJournal {
        dir: PathBuf,
        file: PathBuf,
    }
    impl TempJournal {
        fn create(test_name: &str) -> Self {
            let tick = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir()
                .join(format!("akula-revoke-{test_name}-{}-{tick}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            let file = dir.join("preinstalled-revocation.journal");
            // ONLY the unit-test fixture can provision. The real journal
            // API never calls create, truncate, rename or overwrite.
            let mut handle = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
                .unwrap();
            let mut header = [0u8; HEADER];
            header[0..8].copy_from_slice(&MAGIC);
            header[8..12].copy_from_slice(&VERSION.to_le_bytes());
            header[12..28].copy_from_slice(&ANCHOR);
            handle.write_all(&header).unwrap();
            handle.sync_all().unwrap();
            Self { dir, file }
        }
    }
    impl Drop for TempJournal {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).unwrap();
        }
    }

    fn current_floor(path: &Path, replica: [u8; 16]) -> Result<u64, Hold> {
        let mut file = exclusive_existing(path)?;
        let snapshot = read_log(&mut file, ANCHOR)?;
        Ok(snapshot.floors.get(&replica).copied().unwrap_or(0))
    }

    #[test]
    fn cold_reopen_preserves_denial_floor_not_a_peer_grant() {
        let journal = TempJournal::create("cold");
        assert_eq!(current_floor(&journal.file, REPLICA), Ok(0));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 0), Ok(1));
        assert_eq!(current_floor(&journal.file, REPLICA), Ok(1));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 0), Err(Hold::WrongEpoch));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 1), Ok(2));
        assert_eq!(current_floor(&journal.file, REPLICA), Ok(2));
    }

    #[test]
    fn refuse_missing_file_wrong_installer_binding_and_untrusted_empty_file() {
        let journal = TempJournal::create("binding");
        assert_eq!(
            revoke_existing(&journal.dir.join("missing"), ANCHOR, REPLICA, 0),
            Err(Hold::MissingOrUnreadable)
        );
        assert_eq!(
            revoke_existing(&journal.file, [19; 16], REPLICA, 0),
            Err(Hold::CorruptOrTorn)
        );
        let empty = journal.dir.join("empty");
        File::create(&empty).unwrap();
        assert_eq!(
            revoke_existing(&empty, ANCHOR, REPLICA, 0),
            Err(Hold::CorruptOrTorn)
        );
    }

    #[test]
    fn detect_torn_tail_and_bit_flip_without_automatic_repair() {
        let journal = TempJournal::create("torn");
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 0), Ok(1));
        {
            let mut file = OpenOptions::new().append(true).open(&journal.file).unwrap();
            file.write_all(&[0xfa, 0xfb, 0xfc]).unwrap();
            file.sync_all().unwrap();
        }
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 1), Err(Hold::CorruptOrTorn));
        assert_eq!(current_floor(&journal.file, REPLICA), Err(Hold::CorruptOrTorn));
        let mut damaged = std::fs::read(&journal.file).unwrap();
        damaged.truncate(HEADER + RECORD);
        damaged[HEADER + 10] ^= 0x40;
        std::fs::write(&journal.file, damaged).unwrap();
        assert_eq!(current_floor(&journal.file, REPLICA), Err(Hold::CorruptOrTorn));
    }

    #[test]
    fn different_replica_floors_do_not_erase_each_other() {
        let journal = TempJournal::create("replicas");
        let second = [8u8; 16];
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 0), Ok(1));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, second, 0), Ok(1));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 1), Ok(2));
        assert_eq!(current_floor(&journal.file, REPLICA), Ok(2));
        assert_eq!(current_floor(&journal.file, second), Ok(1));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, second, 0), Err(Hold::WrongEpoch));
    }

    /// Invoked only by the standard Rust libtest binary in a SECOND OS process.
    #[test]
    fn child_process_revoke() {
        let Ok(path) = std::env::var("AKULA_TEST_REVOKE_JOURNAL") else { return };
        assert_eq!(revoke_existing(Path::new(&path), ANCHOR, REPLICA, 0), Ok(1));
    }

    /// Real separate process exits without Rust destructors after a short
    /// durable write, as a crash-like negative fixture (not a power cut).
    #[test]
    fn child_incomplete_record_exit() {
        let Ok(path) = std::env::var("AKULA_TEST_TORN_JOURNAL") else { return };
        let mut f = OpenOptions::new().append(true).open(path).unwrap();
        f.write_all(&[0xaa, 0xbb, 0xcc, 0xdd, 0xee]).unwrap();
        f.sync_all().unwrap();
        std::process::exit(79);
    }

    #[test]
    fn process_exit_during_incomplete_record_requires_manual_review() {
        let journal = TempJournal::create("crash");
        let child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("windows_revocation_journal::tests::child_incomplete_record_exit")
            .env("AKULA_TEST_TORN_JOURNAL", &journal.file)
            .status()
            .unwrap();
        assert_eq!(child.code(), Some(79));
        assert_eq!(current_floor(&journal.file, REPLICA), Err(Hold::CorruptOrTorn));
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 0), Err(Hold::CorruptOrTorn));
    }

    #[test]
    fn old_valid_snapshot_rollback_is_unprotected_and_must_not_issue_grants() {
        let journal = TempJournal::create("rollback");
        let legitimate_old_bytes = std::fs::read(&journal.file).unwrap();
        assert_eq!(revoke_existing(&journal.file, ANCHOR, REPLICA, 0), Ok(1));
        // A powerful attacker who can REPLACE the file between openings
        // can replay a valid old byte-for-byte snapshot. No checksum/CRC
        // can recognize this. The returned zero demonstrates a RELEASE
        // BLOCKER: external installer-protected anti-rollback is required.
        std::fs::write(&journal.file, legitimate_old_bytes).unwrap();
        assert_eq!(current_floor(&journal.file, REPLICA), Ok(0));
        // There is NO access-grant API, so the lowered floor cannot
        // accidentally become File Gate or P2P publication authority.
    }

    #[test]
    fn two_real_os_processes_cannot_both_commit_one_cas_epoch() {
        let journal = TempJournal::create("race");
        let binary = std::env::current_exe().unwrap();
        let spawn_child = || {
            Command::new(&binary)
                .arg("--exact")
                .arg("windows_revocation_journal::tests::child_process_revoke")
                .arg("--nocapture")
                .env("AKULA_TEST_REVOKE_JOURNAL", &journal.file)
                .spawn()
                .unwrap()
        };
        let mut first = spawn_child();
        let mut second = spawn_child();
        let one = first.wait().unwrap().success();
        let two = second.wait().unwrap().success();
        assert_ne!(one, two, "exactly one OS process may win the revoke CAS");
        assert_eq!(current_floor(&journal.file, REPLICA), Ok(1));
    }
}
