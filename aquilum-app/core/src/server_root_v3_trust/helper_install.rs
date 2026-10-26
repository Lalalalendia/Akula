//! B1-E2E-08: administrator-only, one-shot fixed helper publication.
//! This is NOT an SSH/IPC entrypoint, and never executes an ELF, grants
//! Node install authority, changes systemd or repairs a partial transaction.
//! The ONLY production source is the independently root-enrolled signed HOLD.
use super::{bootstrap_helper, open_fixed_trust_fd, root_dir_ok, root_file_ok, stable};
use ring::digest::{Context, SHA256};
use rustix::fs::{open, openat, renameat_with, Mode, OFlags, RenameFlags};
use std::fs::{File, Metadata, Permissions};
use std::io::Write;
use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const STAGED_LEAF: &str = "helper.staged";
const EXECUTABLE_LEAF: &str = "akula-root-b1d0-helper";
const PENDING_LEAF: &str = "akula-root-b1d0-helper.pending";
const MAX_ELF: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum HelperInstallError {
    NotRoot,
    IndependentTrustMissing,
    InvalidSignedStage,
    UnsafeDirectory,
    UnsafeInode,
    ExistingOrInterrupted,
    Io(std::io::Error),
}

impl From<std::io::Error> for HelperInstallError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn os(error: rustix::io::Errno) -> HelperInstallError {
    HelperInstallError::Io(error.into())
}

/// A published file is NOT an activated service or an installation receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstalledHelperHold {
    pub sha256: [u8; 32],
    pub size: u64,
    pub remains_hold: bool,
}

fn dir_at(parent: &File, leaf: &str, mode: u32) -> Result<File, HelperInstallError> {
    let dir = File::from(
        openat(
            parent,
            leaf,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    if !root_dir_ok(&dir.metadata()?, mode) {
        return Err(HelperInstallError::UnsafeDirectory);
    }
    Ok(dir)
}

fn fixed_destination() -> Result<File, HelperInstallError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(HelperInstallError::NotRoot);
    }
    if crate::server_bootstrap_lab::ROOT_HELPER_EXECUTABLE_PATH
        != "/usr/libexec/akula/akula-root-b1d0-helper"
    {
        return Err(HelperInstallError::UnsafeDirectory);
    }
    let root = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    if !root_dir_ok(&root.metadata()?, 0o755) {
        return Err(HelperInstallError::UnsafeDirectory);
    }
    let usr = dir_at(&root, "usr", 0o755)?;
    let libexec = dir_at(&usr, "libexec", 0o755)?;
    // The administrator must separately provision exactly this EMPTY,
    // root-owned 0700 directory. Do not silently create or fix it over SSH.
    dir_at(&libexec, "akula", 0o700)
}

fn staged_from_trust(trust: &File) -> Result<File, HelperInstallError> {
    let parent = dir_at(trust, "..", 0o700)?;
    dir_at(&parent, "bootstrap-v1", 0o700)
}

fn absent(dir: &File, leaf: &str) -> Result<(), HelperInstallError> {
    match openat(
        dir,
        leaf,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        _ => Err(HelperInstallError::ExistingOrInterrupted),
    }
}

fn trusted_file(meta: &Metadata, mode: u32, size: u64) -> bool {
    root_file_ok(meta, mode, MAX_ELF) && size >= 64 && size <= MAX_ELF && meta.len() == size
}

fn hash_elf(file: &File, size: u64) -> Result<[u8; 32], HelperInstallError> {
    let mut context = Context::new(&SHA256);
    let mut header = [0u8; 20];
    let mut block = [0u8; 32 * 1024];
    let mut offset = 0u64;
    while offset < size {
        let want = block.len().min((size - offset) as usize);
        let n = file.read_at(&mut block[..want], offset)?;
        if n == 0 {
            return Err(HelperInstallError::UnsafeInode);
        }
        if offset < header.len() as u64 {
            let copied = n.min(header.len() - offset as usize);
            header[offset as usize..offset as usize + copied]
                .copy_from_slice(&block[..copied]);
        }
        context.update(&block[..n]);
        offset += n as u64;
    }
    let mut extra = [0u8; 1];
    if file.read_at(&mut extra, size)? != 0
        || header[..7] != b"\x7fELF\x02\x01\x01"[..]
        || header[18..20] != b"\x3e\x00"[..]
    {
        return Err(HelperInstallError::UnsafeInode);
    }
    Ok(context.finish().as_ref().try_into().expect("SHA-256 is 32 bytes"))
}

/// Filesystem-only primitive for disposable root-owned fixtures. The
/// production entrypoint below obtains both descriptors and the approval
/// independently, never from a client-controlled argument or SSH frame.
fn publish_signed_hold(
    stage: &File,
    destination: &File,
    verified: &bootstrap_helper::BootstrapHelperHold,
) -> Result<InstalledHelperHold, HelperInstallError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(HelperInstallError::NotRoot);
    }
    if !root_dir_ok(&stage.metadata()?, 0o700)
        || !root_dir_ok(&destination.metadata()?, 0o700)
    {
        return Err(HelperInstallError::UnsafeDirectory);
    }
    if !verified.remains_hold
        || verified.sha256 == [0; 32]
        || verified.release_sequence == 0
        || !(64..=MAX_ELF).contains(&verified.size)
    {
        return Err(HelperInstallError::InvalidSignedStage);
    }
    absent(destination, EXECUTABLE_LEAF)?;
    absent(destination, PENDING_LEAF)?;
    let source = File::from(
        openat(
            stage,
            STAGED_LEAF,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    let before = source.metadata()?;
    if !trusted_file(&before, 0o400, verified.size)
        || hash_elf(&source, verified.size)? != verified.sha256
        || !stable(&before, &source.metadata()?)
    {
        return Err(HelperInstallError::InvalidSignedStage);
    }

    // O_EXCL and RENAME_NOREPLACE make interruption/collision permanent HOLD.
    // Source bytes are copied, NEVER linked to a potentially changing stage.
    let mut pending = File::from(
        openat(
            destination,
            PENDING_LEAF,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| HelperInstallError::ExistingOrInterrupted)?,
    );
    let created = pending.metadata()?;
    if !root_file_ok(&created, 0o600, MAX_ELF) || created.len() != 0 {
        return Err(HelperInstallError::UnsafeInode);
    }
    let mut block = [0u8; 32 * 1024];
    let mut context = Context::new(&SHA256);
    let mut offset = 0u64;
    while offset < verified.size {
        let want = block.len().min((verified.size - offset) as usize);
        let n = source.read_at(&mut block[..want], offset)?;
        if n == 0 {
            return Err(HelperInstallError::UnsafeInode);
        }
        pending.write_all(&block[..n])?;
        context.update(&block[..n]);
        offset += n as u64;
    }
    pending.sync_all()?;
    if context.finish().as_ref() != verified.sha256.as_slice()
        || !stable(&before, &source.metadata()?)
    {
        return Err(HelperInstallError::UnsafeInode);
    }
    pending.set_permissions(Permissions::from_mode(0o500))?;
    pending.sync_all()?;
    let pending_meta = pending.metadata()?;
    if !trusted_file(&pending_meta, 0o500, verified.size) {
        return Err(HelperInstallError::UnsafeInode);
    }
    renameat_with(
        destination,
        PENDING_LEAF,
        destination,
        EXECUTABLE_LEAF,
        RenameFlags::NOREPLACE,
    )
    .map_err(|_| HelperInstallError::ExistingOrInterrupted)?;
    destination.sync_all()?;

    let published = File::from(
        openat(
            destination,
            EXECUTABLE_LEAF,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    let observed = published.metadata()?;
    if !trusted_file(&observed, 0o500, verified.size)
        || pending_meta.dev() != observed.dev()
        || pending_meta.ino() != observed.ino()
        || hash_elf(&published, verified.size)? != verified.sha256
        || !stable(&observed, &published.metadata()?)
    {
        return Err(HelperInstallError::UnsafeInode);
    }
    Ok(InstalledHelperHold {
        sha256: verified.sha256,
        size: verified.size,
        remains_hold: true,
    })
}

/// Independent UID0 administrator command. This is intentionally NOT wired
/// into the helper's stdin parser, native Windows UI, sudo or an SSH command.
/// Requires already enrolled production trust, an independently signed
/// immutable root staging HOLD and an existing fixed root-only destination.
pub fn admin_install_fixed_root_helper_once() -> Result<InstalledHelperHold, HelperInstallError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(HelperInstallError::NotRoot);
    }
    let trust = open_fixed_trust_fd().map_err(|_| HelperInstallError::IndependentTrustMissing)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| HelperInstallError::InvalidSignedStage)?
        .as_secs();
    let verified = bootstrap_helper::inspect_from_trust_fd(&trust, now)
        .map_err(|_| HelperInstallError::InvalidSignedStage)?;
    let stage = staged_from_trust(&trust)?;
    let destination = fixed_destination()?;
    publish_signed_hold(&stage, &destination, &verified)
}

#[cfg(test)]
mod tests;
