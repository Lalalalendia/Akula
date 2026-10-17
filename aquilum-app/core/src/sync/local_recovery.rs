//! SYNC-04g-M1c: read-only, handle-relative inspection of local publications.
//!
//! A SQLite attempt is *not* proof that any filesystem mutation occurred.
//! These diagnostics neither release durable publication claims nor authorize
//! replay, deletion, quarantine moves, remote baseline commits or local writes.
//! Even two matching inspections are NOT an atomic filesystem snapshot against
//! external processes. They are evidence for a later explicit recovery policy.

use super::pinned_read::PinnedFileIdentity;
use super::{OperationId, VaultPortError};
use std::fs::File;
use std::io::Read;

#[cfg(unix)]
use rustix::fs::{openat, Mode, OFlags};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ObservedLocalFile {
    Absent,
    Present {
        identity: PinnedFileIdentity,
        byte_length: u64,
        matches_expected_hash: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalRecoveryAssessment {
    /// An attempt exists, but neither name is visible during inspection.
    NoVisibleNames,
    /// Staged name holds expected bytes; no destination is visible.
    StagedOnly,
    /// Final name holds expected bytes; NOT proof it came from this operation.
    FinalNameMatchesExpected,
    /// Stage and final both exist; provenance/collision needs review.
    BothNamesPresent,
    /// Final name holds a different file: do not replace it.
    DifferentFinalFile,
    /// Staged name does not match the operation's expected content.
    UntrustedStage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReadOnlyLocalPublicationEvidence {
    pub operation_id: OperationId,
    pub parent_identity: PinnedFileIdentity,
    pub stage: ObservedLocalFile,
    pub final_name: ObservedLocalFile,
    pub assessment: LocalRecoveryAssessment,
}

fn file_identity(file: &File) -> Result<PinnedFileIdentity, VaultPortError> {
    let meta = file.metadata().map_err(|e| VaultPortError::Io(e.to_string()))?;
    if !meta.is_file() {
        return Err(VaultPortError::UnsafePath);
    }
    #[cfg(unix)]
    {
        if meta.ino() == 0 || meta.nlink() != 1 {
            return Err(VaultPortError::UnsafePath);
        }
        Ok(PinnedFileIdentity { device: meta.dev(), file: meta.ino() })
    }
    #[cfg(windows)]
    {
        let (volume, number, links) =
            super::pinned_windows::file_identity_with_links(file)?;
        if links != 1 {
            return Err(VaultPortError::UnsafePath);
        }
        Ok(PinnedFileIdentity { device: u64::from(volume), file: number })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(VaultPortError::PermissionDenied)
    }
}

fn open_optional_file(parent: &File, name: &str)
    -> Result<Option<File>, VaultPortError>
{
    if name.is_empty() || name == "." || name == ".."
        || name.chars().any(|ch| matches!(ch, '/' | '\\' | ':' | '\0'))
    {
        return Err(VaultPortError::UnsafePath);
    }
    #[cfg(unix)]
    {
        match openat(
            parent, name, OFlags::RDONLY | OFlags::NOFOLLOW
                | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(handle) => Ok(Some(File::from(handle))),
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(error) => Err(VaultPortError::Io(error.to_string())),
        }
    }
    #[cfg(windows)]
    {
        super::pinned_windows::open_optional_child(parent, name)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent;
        Err(VaultPortError::PermissionDenied)
    }
}

fn inspect_child(parent: &File, name: &str, expected_hash: &str, max_bytes: u64)
    -> Result<ObservedLocalFile, VaultPortError>
{
    const HARD_LIMIT: u64 = 32 * 1024 * 1024;
    if max_bytes == 0 || max_bytes > HARD_LIMIT {
        return Err(VaultPortError::PermissionDenied);
    }
    let Some(mut file) = open_optional_file(parent, name)? else {
        return Ok(ObservedLocalFile::Absent);
    };
    let before = file.metadata().map_err(|e| VaultPortError::Io(e.to_string()))?;
    let id = file_identity(&file)?;
    if before.len() > max_bytes {
        return Err(VaultPortError::PermissionDenied);
    }
    let mut hasher = blake3::Hasher::new();
    let mut total = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let count = (&mut file).take(
            (max_bytes - total).saturating_add(1).min(buf.len() as u64),
        ).read(&mut buf).map_err(|e| VaultPortError::Io(e.to_string()))?;
        if count == 0 { break; }
        total = total.checked_add(count as u64)
            .ok_or(VaultPortError::PermissionDenied)?;
        if total > max_bytes {
            return Err(VaultPortError::PermissionDenied);
        }
        hasher.update(&buf[..count]);
    }
    let after = file.metadata().map_err(|e| VaultPortError::Io(e.to_string()))?;
    if before.len() != total || after.len() != total
        || before.modified().ok() != after.modified().ok()
        || file_identity(&file)? != id
    {
        return Err(VaultPortError::FileChanged);
    }
    Ok(ObservedLocalFile::Present {
        identity: id,
        byte_length: total,
        matches_expected_hash: hasher.finalize().to_hex().as_str() == expected_hash,
    })
}

pub(super) fn inspect_names(
    parent: &File, operation: OperationId, leaf: &str,
    expected_hash: &str, max_bytes: u64,
) -> Result<(ObservedLocalFile, ObservedLocalFile), VaultPortError> {
    let stage_name = format!(".akula-sync-stage-{operation}");
    let stage = inspect_child(parent, &stage_name, expected_hash, max_bytes)?;
    let final_name = inspect_child(parent, leaf, expected_hash, max_bytes)?;
    Ok((stage, final_name))
}

pub(super) fn classify(
    stage: &ObservedLocalFile, final_name: &ObservedLocalFile,
) -> LocalRecoveryAssessment {
    use LocalRecoveryAssessment::*;
    use ObservedLocalFile::*;
    if matches!(stage, Present { matches_expected_hash: false, .. }) {
        return UntrustedStage;
    }
    if matches!(final_name, Present { matches_expected_hash: false, .. }) {
        return DifferentFinalFile;
    }
    match (stage, final_name) {
        (Absent, Absent) => NoVisibleNames,
        (Present { .. }, Absent) => StagedOnly,
        (Absent, Present { .. }) => FinalNameMatchesExpected,
        (Present { .. }, Present { .. }) => BothNamesPresent,
    }
}
