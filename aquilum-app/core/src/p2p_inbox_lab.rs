//! Explicit receiver "Save as NEW file" from a pinned, on-disk verified stage.
//!
//! This optional manual P2P inbox path is deliberately NOT Akula Vault sync,
//! File Gate application or an ACK. The caller must independently trust the
//! expected root and size from an approved OfferSession. This code checks the
//! actual opened stage bytes again and NEVER replaces an existing destination.
//! No server, transport, UI action or unattended write is started here.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const MAX_INBOX_ITEM_BYTES: u64 = 8 * 1024 * 1024;
const COPY_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum InboxError {
    InvalidManifest,
    InvalidDestination,
    NotRegularStage,
    SizeMismatch,
    DigestMismatch,
    AlreadyExists,
    Io(io::Error),
    FileGate(crate::files::error::FileCommandError),
}

impl From<io::Error> for InboxError {
    fn from(error: io::Error) -> Self {
        if error.kind() == io::ErrorKind::AlreadyExists {
            Self::AlreadyExists
        } else {
            Self::Io(error)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewInboxFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub blake3_root: [u8; 32],
}

/// Keep a failed/incomplete candidate from being mistaken for a saved file.
/// The independent destination name only becomes visible after a full digest
/// match and an atomic no-replace hard-link to the completely written candidate.
struct RemoveScratch(PathBuf);

impl Drop for RemoveScratch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Save exactly one user-accepted, hash-pinned staged file at a user-selected
/// NEW path. The caller retains ownership of the opened `File` and MUST obtain
/// the expected root/size from independently authenticated Akula consent,
/// never from a remotely supplied display name or staged file itself.
///
/// * Reads from one already-opened regular-file handle; no second stage-path
///   lookup, no unbounded allocation (64 KiB scratch buffer).
/// * Does not publish a destination until the entire physical stage has been
///   copied, rehashed and synced to a private new sibling candidate.
/// * `hard_link` is a same-directory atomic create-NO-REPLACE publication;
///   existing files and symlinks cannot be overwritten, including under races.
/// * It refuses unsupported hard-link filesystems rather than falling back
///   to unsafe rename-overwrite.
/// * This is NOT a power-loss durable receipt, protected destination-dir
///   custody or a trusted Vault File Gate write. Never advertise APPLIED/ACK.
pub fn save_verified_stage_as_new(
    stage: &mut File,
    expected_bytes: u64,
    expected_root: [u8; 32],
    destination: &Path,
) -> Result<NewInboxFile, InboxError> {
    if expected_bytes == 0 || expected_bytes > MAX_INBOX_ITEM_BYTES || expected_root == [0; 32] {
        return Err(InboxError::InvalidManifest);
    }
    let Some(name) = destination.file_name() else {
        return Err(InboxError::InvalidDestination);
    };
    if name.is_empty() {
        return Err(InboxError::InvalidDestination);
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // A caller chooses the destination. Reject a symlinked immediate parent,
    // but do not claim that all ancestor path races are solved by this check.
    let parent_info = fs::symlink_metadata(parent)?;
    if !parent_info.is_dir() || parent_info.file_type().is_symlink() {
        return Err(InboxError::InvalidDestination);
    }
    // Catch existing destinations including dangling symlinks. hard_link below
    // enforces the same no-replace rule atomically if another writer races us.
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(InboxError::AlreadyExists),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    let before = stage.metadata()?;
    if !before.is_file() {
        return Err(InboxError::NotRegularStage);
    }
    if before.len() != expected_bytes {
        return Err(InboxError::SizeMismatch);
    }
    stage.seek(SeekFrom::Start(0))?;
    let scratch_path = parent.join(format!(".akula-inbox-{}.part", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut scratch = options.open(&scratch_path)?;
    let _remove_if_incomplete = RemoveScratch(scratch_path.clone());

    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut hasher = blake3::Hasher::new();
    let mut copied = 0u64;
    loop {
        let got = stage.read(&mut buffer)?;
        if got == 0 {
            break;
        }
        copied = copied
            .checked_add(got as u64)
            .ok_or(InboxError::SizeMismatch)?;
        if copied > expected_bytes {
            return Err(InboxError::SizeMismatch);
        }
        hasher.update(&buffer[..got]);
        scratch.write_all(&buffer[..got])?;
    }
    if copied != expected_bytes || stage.metadata()?.len() != expected_bytes {
        return Err(InboxError::SizeMismatch);
    }
    if *hasher.finalize().as_bytes() != expected_root {
        return Err(InboxError::DigestMismatch);
    }
    scratch.sync_all()?;
    drop(scratch);

    fs::hard_link(&scratch_path, destination).map_err(InboxError::from)?;
    Ok(NewInboxFile {
        path: destination.to_path_buf(),
        bytes: expected_bytes,
        blake3_root: expected_root,
    })
}


/// A separate receiver publication route using Akula's EXISTING Core File Gate.
/// This is the only path from this P2P lab intended for a managed Akula file.
/// The expected manifest must come from independently authenticated, approved
/// Akula consent; a network label is never a local destination path.
///
/// Bounded verified bytes are handed to gate::create_binary, which provides
/// the regular Core mutation/admission policy and create-new semantics. Unlike
/// the sibling hard-link preview, the existing File Gate may briefly expose a
/// partially written new destination before sync_all completes. It is NOT
/// crash-atomic, host-enrolled sync authority, Vault apply or durable ACK.
pub fn save_verified_stage_through_file_gate(
    core: &crate::Core,
    stage: &mut File,
    expected_bytes: u64,
    expected_root: [u8; 32],
    destination: &Path,
) -> Result<NewInboxFile, InboxError> {
    if expected_bytes == 0 || expected_bytes > MAX_INBOX_ITEM_BYTES || expected_root == [0; 32] {
        return Err(InboxError::InvalidManifest);
    }
    if destination.file_name().is_none() {
        return Err(InboxError::InvalidDestination);
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_info = fs::symlink_metadata(parent)?;
    if !parent_info.is_dir() || parent_info.file_type().is_symlink() {
        return Err(InboxError::InvalidDestination);
    }

    // Retain a best-effort precheck, but the ONLY authoritative no-clobber
    // decision is File Gate's O_EXCL / CREATE_NEW underneath its mutation guard.
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(InboxError::AlreadyExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let before = stage.metadata()?;
    if !before.is_file() {
        return Err(InboxError::NotRegularStage);
    }
    if before.len() != expected_bytes {
        return Err(InboxError::SizeMismatch);
    }
    stage.seek(SeekFrom::Start(0))?;

    // Eight MiB is an absolute per-item bound; never buffer an unbounded
    // network payload or trust the stage filename as evidence of ownership.
    let mut verified = Vec::with_capacity(expected_bytes as usize);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; COPY_BUFFER_BYTES];
    let mut copied = 0u64;
    loop {
        let got = stage.read(&mut buffer)?;
        if got == 0 {
            break;
        }
        copied = copied
            .checked_add(got as u64)
            .ok_or(InboxError::SizeMismatch)?;
        if copied > expected_bytes {
            return Err(InboxError::SizeMismatch);
        }
        hasher.update(&buffer[..got]);
        verified.extend_from_slice(&buffer[..got]);
    }
    if copied != expected_bytes || stage.metadata()?.len() != expected_bytes {
        return Err(InboxError::SizeMismatch);
    }
    if *hasher.finalize().as_bytes() != expected_root {
        return Err(InboxError::DigestMismatch);
    }

    match crate::files::gate::create_binary(core, destination, &verified) {
        Ok(_) => Ok(NewInboxFile {
            path: destination.to_path_buf(),
            bytes: expected_bytes,
            blake3_root: expected_root,
        }),
        Err(crate::files::error::FileCommandError::AlreadyExists { .. }) => {
            Err(InboxError::AlreadyExists)
        }
        Err(error) => Err(InboxError::FileGate(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opened(root: &Path, bytes: &[u8]) -> File {
        let path = root.join("private-stage.bin");
        fs::write(&path, bytes).unwrap();
        File::open(path).unwrap()
    }

    #[test]
    fn creates_new_verified_file_with_bounded_copy_and_exact_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = (0..(64 * 1024 + 23))
            .map(|i| ((i * 31 + 5) % 251) as u8)
            .collect::<Vec<_>>();
        let mut stage = opened(dir.path(), &bytes);
        let output = dir.path().join("user-chosen-name.bin");
        let expected = *blake3::hash(&bytes).as_bytes();
        let receipt =
            save_verified_stage_as_new(&mut stage, bytes.len() as u64, expected, &output).unwrap();
        assert_eq!(receipt.path, output);
        assert_eq!(receipt.bytes, bytes.len() as u64);
        assert_eq!(receipt.blake3_root, expected);
        assert_eq!(fs::read(&output).unwrap(), bytes);
        assert_eq!(
            fs::read(dir.path().join("private-stage.bin")).unwrap(),
            bytes
        );
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn a_preexisting_destination_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let mut stage = opened(dir.path(), b"selected bytes");
        let output = dir.path().join("keep.bin");
        fs::write(&output, b"my existing document").unwrap();
        let outcome = save_verified_stage_as_new(
            &mut stage,
            14,
            *blake3::hash(b"selected bytes").as_bytes(),
            &output,
        );
        assert!(matches!(outcome, Err(InboxError::AlreadyExists)));
        assert_eq!(fs::read(&output).unwrap(), b"my existing document");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn wrong_root_and_changed_size_never_publish_partial_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut stage = opened(dir.path(), b"byte-for-byte sample");
        let output = dir.path().join("never-created.bin");
        let bad_hash = *blake3::hash(b"forged commitment").as_bytes();
        assert!(matches!(
            save_verified_stage_as_new(&mut stage, 20, bad_hash, &output),
            Err(InboxError::DigestMismatch)
        ));
        assert!(!output.exists());
        assert!(matches!(
            save_verified_stage_as_new(&mut stage, 19, bad_hash, &output),
            Err(InboxError::SizeMismatch)
        ));
        assert!(!output.exists());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn zero_or_oversized_manifest_is_refused_before_publication() {
        let dir = tempfile::tempdir().unwrap();
        let mut stage = opened(dir.path(), b"stage bytes");
        let target = dir.path().join("invalid.bin");
        let hash = *blake3::hash(b"stage bytes").as_bytes();
        assert!(matches!(
            save_verified_stage_as_new(&mut stage, 0, hash, &target),
            Err(InboxError::InvalidManifest)
        ));
        assert!(matches!(
            save_verified_stage_as_new(&mut stage, MAX_INBOX_ITEM_BYTES + 1, hash, &target),
            Err(InboxError::InvalidManifest)
        ));
        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn stage_path_swap_after_open_cannot_change_published_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("private-stage.bin");
        fs::write(&source, b"pinned original data").unwrap();
        let mut opened_stage = File::open(&source).unwrap();
        fs::rename(&source, dir.path().join("renamed-stage.bin")).unwrap();
        fs::write(&source, b"attacker replacement").unwrap();
        let output = dir.path().join("correct-copy.bin");
        save_verified_stage_as_new(
            &mut opened_stage,
            20,
            *blake3::hash(b"pinned original data").as_bytes(),
            &output,
        )
        .unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"pinned original data");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_destination_parent_is_refused() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        let parent_link = dir.path().join("redirect");
        symlink(&outside, &parent_link).unwrap();
        let mut stage = opened(dir.path(), b"trusted");
        let target = parent_link.join("unexpected.bin");
        assert!(matches!(
            save_verified_stage_as_new(
                &mut stage,
                7,
                *blake3::hash(b"trusted").as_bytes(),
                &target
            ),
            Err(InboxError::InvalidDestination)
        ));
        assert!(!outside.join("unexpected.bin").exists());
    }
    #[test]
    fn file_gate_receives_only_verified_bytes_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = dir.path().join("appdata");
        fs::create_dir(&app_data).unwrap();
        let core = crate::Core::open(
            &app_data,
            std::sync::Arc::new(|_: crate::CoreEvent| {}),
        );
        let bytes = b"actual verified stage data";
        let mut stage = opened(dir.path(), bytes);
        let root = *blake3::hash(bytes).as_bytes();
        let target = dir.path().join("received.bin");

        let result = save_verified_stage_through_file_gate(
            &core,
            &mut stage,
            bytes.len() as u64,
            root,
            &target,
        )
        .unwrap();
        assert_eq!(result.bytes, bytes.len() as u64);
        assert_eq!(result.blake3_root, root);
        assert_eq!(fs::read(&target).unwrap(), bytes);

        assert!(matches!(
            save_verified_stage_through_file_gate(
                &core,
                &mut stage,
                bytes.len() as u64,
                root,
                &target,
            ),
            Err(InboxError::AlreadyExists)
        ));
        assert_eq!(fs::read(&target).unwrap(), bytes);

        let untouched = dir.path().join("wrong-root.bin");
        assert!(matches!(
            save_verified_stage_through_file_gate(
                &core,
                &mut stage,
                bytes.len() as u64,
                *blake3::hash(b"forgery").as_bytes(),
                &untouched,
            ),
            Err(InboxError::DigestMismatch)
        ));
        assert!(!untouched.exists());
        core.shutdown();
    }

}
