//! Per-offer, append-only Windows NSIS retry tickets.
//! Exact signed identity: stable three-part version and lowercase SHA-256.
//! Never overwrite a previous ticket, nor run an update if state is unwritable.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::Path;

const DIRECTORY: &str = "signed-update-attempts";
const MAX_ATTEMPTS: usize = 2;

fn valid_version(value: &str) -> bool {
    if value.is_empty() || value.len() > 32 {
        return false;
    }
    let parts: Vec<_> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.len() <= 10
                && (part.len() == 1 || !part.starts_with('0'))
                && part.bytes().all(|b| b.is_ascii_digit())
        })
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn ticket_name(version: &str, sha256: &str, slot: usize) -> String {
    format!("v{version}-{sha256}-{slot}.attempt")
}

/// Record an admitted, publisher-signed install attempt before handing control
/// to NSIS. An exclusive create is a cross-process ticket: at most two calls
/// can succeed for a given immutable payload, including across app restarts.
/// A crash after creation consumes the ticket conservatively (fail closed).
pub(super) fn record(data_dir: &Path, version: &str, sha256: &str) -> Result<(), String> {
    if !valid_version(version) || !valid_sha256(sha256) {
        return Err("Akula updater retry identity failed validation".into());
    }

    let directory = data_dir.join(DIRECTORY);
    fs::create_dir_all(&directory)
        .map_err(|e| format!("cannot create Akula updater retry directory: {e}"))?;
    let metadata = fs::symlink_metadata(&directory)
        .map_err(|e| format!("cannot inspect Akula updater retry directory: {e}"))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Akula updater retry directory is not a real directory".into());
    }

    for slot in 1..=MAX_ATTEMPTS {
        let path = directory.join(ticket_name(version, sha256, slot));
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => {
                // A partially written ticket is still consumed, not retried.
                file.write_all(b"Akula signed NSIS attempt v1\n")
                    .and_then(|()| file.sync_all())
                    .map_err(|e| format!("cannot persist Akula updater attempt: {e}"))?;
                return Ok(());
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot claim Akula updater attempt: {e}")),
        }
    }

    Err(format!(
        "Akula update {version} has reached {MAX_ATTEMPTS} attempts; install the verified release manually"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    const VERSION: &str = "0.3.19";
    const SHA: &str = "a123456789abcdefa123456789abcdefa123456789abcdefa123456789abcdef";

    #[test]
    fn attempts_are_durable_and_scoped_to_exact_signed_release() {
        let dir = tempfile::tempdir().unwrap();
        assert!(record(dir.path(), VERSION, SHA).is_ok());
        assert!(record(dir.path(), VERSION, SHA).is_ok());
        assert!(record(dir.path(), VERSION, SHA).unwrap_err().contains("manually"));
        assert!(record(dir.path(), "0.3.20", SHA).is_ok());
        let other_sha = "b".repeat(64);
        assert!(record(dir.path(), VERSION, &other_sha).is_ok());

        let first = dir.path().join(DIRECTORY).join(ticket_name(VERSION, SHA, 1));
        assert!(first.is_file());
        assert_eq!(fs::read(first).unwrap(), b"Akula signed NSIS attempt v1\n");
    }

    #[test]
    fn invalid_identity_or_unwritable_marker_directory_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        for version in ["", "v0.3.19", "../tmp", "0.3.019", "0.3.19.1"] {
            assert!(record(dir.path(), version, SHA).is_err());
        }
        assert!(record(dir.path(), VERSION, &"G".repeat(64)).is_err());
        fs::write(dir.path().join(DIRECTORY), b"not a directory").unwrap();
        assert!(record(dir.path(), VERSION, SHA).is_err());
    }

    #[test]
    fn concurrent_process_style_claims_cannot_exceed_two_slots() {
        let dir = tempfile::tempdir().unwrap();
        let barrier = Arc::new(Barrier::new(17));
        let mut workers = Vec::new();
        for _ in 0..16 {
            let barrier = Arc::clone(&barrier);
            let path = dir.path().to_owned();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                record(&path, VERSION, SHA)
            }));
        }
        barrier.wait();
        let successes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|result| result.is_ok())
            .count();
        assert_eq!(successes, 2);
        assert!(record(dir.path(), VERSION, SHA).is_err());
    }
}
