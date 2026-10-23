//! Receiver-local durable-enough-for-process-restart Inbox, NOT a signed
//! delivery ACK or proof of OS-owned Windows path custody.
//! The only authority here is a previously authenticated, explicitly requested
//! one-file QUIC reception. No sender-chosen names or Vault paths are accepted.

use akula_inbox_ledger_lab::{ExpectedTransfer, InboxLedger, LedgerError, LocalStatus, Transport};
use aquilum_core::p2p_inbox_lab::{InboxError, save_verified_stage_as_new};
#[cfg(not(windows))]
use std::ffi::OsString;
use std::{
    fs::{self, File},
    io,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Consistent local-only root that a future Akula Native Inbox can read.
/// Not taken from any QUIC frame, URL, remote label or provider metadata.
pub(super) fn default_root() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA is not available for this user")?;
    #[cfg(not(windows))]
    let base = {
        let selected: OsString = match std::env::var_os("XDG_DATA_HOME") {
            Some(value) => value,
            None => {
                let home = std::env::var_os("HOME").ok_or("HOME is not available")?;
                PathBuf::from(home).join(".local/share").into_os_string()
            }
        };
        PathBuf::from(selected)
    };
    if !base.is_absolute() {
        return Err("receiver app data base must be an absolute local path".into());
    }
    let root = base.join("Akula").join("received-inbox");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(&root)
            .map_err(|error| format!("create private receiver Inbox: {error}"))?;
        let metadata = fs::symlink_metadata(&root)
            .map_err(|error| format!("private receiver Inbox metadata: {error}"))?;
        if !metadata.is_dir() || (metadata.permissions().mode() & 0o077) != 0 {
            return Err("receiver Inbox root is not an exclusive owner-only directory".into());
        }
    }
    #[cfg(windows)]
    fs::create_dir_all(&root).map_err(|error| format!("create receiver Inbox: {error}"))?;
    Ok(root)
}

/// Until the signed Offer UUID exists, duplicate content from one authenticated
/// sender to the same receiver intentionally maps to the same local object.
fn expected_transfer(
    size: u64,
    digest: [u8; 32],
    sender: [u8; 32],
    receiver: [u8; 32],
) -> ExpectedTransfer {
    let mut hash = blake3::Hasher::new_derive_key("Akula one-file LAN beta received content v1");
    hash.update(&sender);
    hash.update(&receiver);
    hash.update(&size.to_be_bytes());
    hash.update(&digest);
    let id_hash = hash.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&id_hash.as_bytes()[..16]);
    id[6] = (id[6] & 0x0f) | 0x80; // Deterministic UUID version 8
    id[8] = (id[8] & 0x3f) | 0x80;
    ExpectedTransfer {
        id: Uuid::from_bytes(id),
        sender_key: sender,
        recipient_key: receiver,
        source: Transport::P2p,
        bytes: size,
        blake3: digest,
    }
}

pub(super) fn persist_received(
    pinned_stage: &mut File,
    inbox_root: &Path,
    size: u64,
    digest: [u8; 32],
    authenticated_sender: [u8; 32],
    local_receiver: [u8; 32],
) -> Result<ExpectedTransfer, String> {
    let intent = expected_transfer(size, digest, authenticated_sender, local_receiver);
    let mut ledger = InboxLedger::open(inbox_root)
        .map_err(|error| format!("open receiver Inbox ledger: {error:?}"))?;
    // Already checked by the real QUIC TLS identity boundary. Metadata from
    // network frames alone is not a sender device/recipient grant.
    let state = ledger
        .receive_with_writer(&intent, |destination| {
            save_verified_stage_as_new(pinned_stage, size, digest, destination)
                .map(|_| ())
                .map_err(|error| match error {
                    InboxError::Io(source) => LedgerError::Io(source),
                    InboxError::AlreadyExists => {
                        LedgerError::Io(io::Error::from(io::ErrorKind::AlreadyExists))
                    }
                    _ => LedgerError::InvalidLocalObject,
                })
        })
        .map_err(|error| format!("Inbox rejected local object: {error:?}"))?;
    if state != LocalStatus::VerifiedLocal {
        return Err("receiver Inbox refused an unverified local object".into());
    }
    Ok(intent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage(root: &Path, data: &[u8]) -> File {
        let path = root.join("verified-stage.bin");
        fs::write(&path, data).unwrap();
        File::open(path).unwrap()
    }

    #[test]
    fn cold_received_inbox_replays_only_one_verified_object() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"one first-beta authenticated local file";
        let mut file = stage(dir.path(), bytes);
        let digest = *blake3::hash(bytes).as_bytes();
        let root = dir.path().join("inbox");
        let sender = [11u8; 32];
        let receiver = [12u8; 32];
        let first = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            sender,
            receiver,
        )
        .unwrap();
        let replay = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            sender,
            receiver,
        )
        .unwrap();
        assert_eq!(first, replay);
        let cold = InboxLedger::open(&root).unwrap();
        assert_eq!(cold.count().unwrap(), 1);
        assert_eq!(cold.status(first.id).unwrap(), LocalStatus::VerifiedLocal);
        assert_eq!(fs::read(cold.object_path(first.id)).unwrap(), bytes);
    }

    #[test]
    fn altered_local_received_object_fails_closed_on_retry() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"valid object before tampering";
        let mut file = stage(dir.path(), bytes);
        let digest = *blake3::hash(bytes).as_bytes();
        let root = dir.path().join("inbox");
        let sender = [17u8; 32];
        let receiver = [18u8; 32];
        let intent = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            sender,
            receiver,
        )
        .unwrap();
        let ledger = InboxLedger::open(&root).unwrap();
        let path = ledger.object_path(intent.id);
        fs::write(&path, b"tampered").unwrap();
        drop(ledger);
        assert!(
            persist_received(
                &mut file,
                &root,
                bytes.len() as u64,
                digest,
                sender,
                receiver,
            )
            .is_err()
        );
        let cold = InboxLedger::open(&root).unwrap();
        assert_eq!(cold.status(intent.id).unwrap(), LocalStatus::NeedsReview);
        assert_eq!(cold.count().unwrap(), 1);
        assert_eq!(fs::read(path).unwrap(), b"tampered");
    }

    #[test]
    fn different_sender_does_not_collide_on_identical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"duplicate bytes from different devices";
        let mut file = stage(dir.path(), bytes);
        let digest = *blake3::hash(bytes).as_bytes();
        let root = dir.path().join("inbox");
        let first = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            [1; 32],
            [2; 32],
        )
        .unwrap();
        let second = persist_received(
            &mut file,
            &root,
            bytes.len() as u64,
            digest,
            [3; 32],
            [2; 32],
        )
        .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(InboxLedger::open(&root).unwrap().count().unwrap(), 2);
    }
}
