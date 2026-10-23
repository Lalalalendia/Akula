//! Opt-in Akula Core File Gate writer for a cold-verified Staya Bao stage.
//! It is a create-new laboratory, not an atomic publisher or durable receipt.
use super::*;

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
    fn file_gate_receives_only_verified_bytes_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let app_data = dir.path().join("appdata");
        fs::create_dir(&app_data).unwrap();
        let core = crate::Core::open(&app_data, std::sync::Arc::new(|_: crate::CoreEvent| {}));
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
