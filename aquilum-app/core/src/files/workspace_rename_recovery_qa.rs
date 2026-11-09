//! Test-only recovery classification for whole-workspace folder renames.
//!
//! No runtime caller, filesystem move, SQLite write, journal cleanup, or
//! automatic replay exists here. A journal outside the vault is useful evidence
//! of a crash window, NOT an authenticated host lease or durability guarantee.
//! A cooperative or malicious writer can still replace pathnames between
//! observations, so every ambiguous state is HOLD.
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const SCHEMA: u32 = 1;
const MAX_RECORD_BYTES: u64 = 4_096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct PhysicalRootId {
    device: u64,
    file: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameIntent {
    schema: u32,
    operation_id: Uuid,
    workspace_id: Uuid,
    source: PathBuf,
    destination: PathBuf,
    expected_root: PhysicalRootId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecoveryView {
    Prepared,
    MovedWaitingForRegistry,
    RegistryChangedWaitingForAck,
    Hold,
}

fn physical_root(path: &Path) -> io::Result<Option<PhysicalRootId>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(io::Error::new(ErrorKind::InvalidData, "invalid root type"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x0000_0400 != 0 {
            return Err(io::Error::new(ErrorKind::InvalidData, "root is a reparse point"));
        }
        let device = meta.volume_serial_number().ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "volume ID unavailable")
        })?;
        let file = meta.file_index().ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "file ID unavailable")
        })?;
        if file == 0 { return Err(io::Error::new(ErrorKind::InvalidData, "missing file ID")); }
        return Ok(Some(PhysicalRootId { device: u64::from(device), file }));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.ino() == 0 {
            return Err(io::Error::new(ErrorKind::InvalidData, "missing inode"));
        }
        return Ok(Some(PhysicalRootId { device: meta.dev(), file: meta.ino() }));
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = meta;
        Err(io::Error::new(ErrorKind::Unsupported, "physical ID not implemented"))
    }
}

fn valid_intent(intent: &RenameIntent) -> bool {
    let source = &intent.source;
    let dest = &intent.destination;
    intent.schema == SCHEMA
        && intent.operation_id != Uuid::nil()
        && intent.workspace_id != Uuid::nil()
        && source.is_absolute() && dest.is_absolute()
        && source != dest
        && source.parent().is_some()
        && source.parent() == dest.parent()
        && source.file_name().is_some()
        && dest.file_name().is_some()
        && !source.components().any(|c| matches!(c, std::path::Component::CurDir | std::path::Component::ParentDir))
        && !dest.components().any(|c| matches!(c, std::path::Component::CurDir | std::path::Component::ParentDir))
        && intent.expected_root.file != 0
}

fn invalid(reason: &'static str) -> io::Error {
    io::Error::new(ErrorKind::InvalidData, reason)
}

fn write_new_journal(path: &Path, intent: &RenameIntent) -> io::Result<()> {
    if !valid_intent(intent) { return Err(invalid("malformed rename intent")); }
    if !path.is_absolute() { return Err(invalid("journal path is not absolute")); }
    let parent = path.parent().ok_or_else(|| invalid("no journal parent"))?;
    if parent.starts_with(&intent.source) || parent.starts_with(&intent.destination) {
        return Err(invalid("journal must be outside both vault roots"));
    }
    let contents = serde_json::to_vec(intent).map_err(|_| invalid("intent serialization failed"))?;
    if contents.len() as u64 > MAX_RECORD_BYTES {
        return Err(invalid("intent exceeds record limit"));
    }
    // This is a fixture-only primitive. create_new refuses duplicate operation
    // names and never truncates/replaces an existing journal.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&contents)?;
    file.sync_all()?;
    drop(file);
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    // On Windows, no certified directory flush is implemented: don't claim
    // power-loss durability based on FlushFileBuffers of the journal alone.
    Ok(())
}

fn read_journal(path: &Path) -> io::Result<RenameIntent> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > MAX_RECORD_BYTES {
        return Err(invalid("unsafe journal object"));
    }
    #[cfg(windows)]
    if {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x0000_0400 != 0
    } {
        return Err(invalid("journal is a reparse point"));
    }
    let mut content = Vec::new();
    File::open(path)?.take(MAX_RECORD_BYTES + 1).read_to_end(&mut content)?;
    if content.len() as u64 > MAX_RECORD_BYTES { return Err(invalid("oversize journal")); }
    let intent: RenameIntent =
        serde_json::from_slice(&content).map_err(|_| invalid("invalid journal encoding"))?;
    if !valid_intent(&intent) { return Err(invalid("invalid journal schema or path")); }
    Ok(intent)
}

/// Observational only: NEVER calls rename, deletes a folder, updates SQLite
/// or advances/clears a journal. Any partial/ambiguous filesystem state HOLDs.
fn classify(intent: &RenameIntent, registered: Option<(Uuid, &Path)>) -> RecoveryView {
    if !valid_intent(intent) { return RecoveryView::Hold; }
    let Some((workspace_id, registered_path)) = registered else { return RecoveryView::Hold };
    if workspace_id != intent.workspace_id { return RecoveryView::Hold; }
    let old_registry = registered_path == intent.source;
    let new_registry = registered_path == intent.destination;
    if !old_registry && !new_registry { return RecoveryView::Hold; }

    let old = match physical_root(&intent.source) {
        Ok(value) => value,
        Err(_) => return RecoveryView::Hold,
    };
    let new = match physical_root(&intent.destination) {
        Ok(value) => value,
        Err(_) => return RecoveryView::Hold,
    };
    match (old, new, old_registry, new_registry) {
        (Some(root), None, true, false) if root == intent.expected_root => RecoveryView::Prepared,
        (None, Some(root), true, false) if root == intent.expected_root => RecoveryView::MovedWaitingForRegistry,
        (None, Some(root), false, true) if root == intent.expected_root => RecoveryView::RegistryChangedWaitingForAck,
        _ => RecoveryView::Hold,
    }
}

fn classify_journal(path: &Path, registered: Option<(Uuid, &Path)>) -> RecoveryView {
    match read_journal(path) {
        Ok(intent) => classify(&intent, registered),
        Err(_) => RecoveryView::Hold,
    }
}

#[cfg(test)]
mod tests;
