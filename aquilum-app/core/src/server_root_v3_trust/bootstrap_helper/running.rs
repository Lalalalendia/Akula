//! Read-only provenance of the actual UID0 root-helper process image.
//! A signed staged ELF is NOT proof that the current root process runs it.
//! Only an independently installed, root-owned fixed executable inode may
//! be compared against /proc/self/exe (kernel-owned procfs reference).
//! Never executes the staged helper, creates directories or grants IPC rights.
use super::{BootstrapHelperHold, MAX_HELPER_BYTES};
use ring::digest::{Context, SHA256};
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::{File, Metadata};
use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
use std::path::Path;

const EXEC_LEAF: &str = "akula-root-b1d0-helper";
const EXEC_MODE: u32 = 0o500;

#[derive(Debug)]
pub(super) enum RunningImageError {
    NotRoot,
    UntrustedImage,
    Io(std::io::Error),
}
impl From<std::io::Error> for RunningImageError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
fn os(error: rustix::io::Errno) -> RunningImageError {
    RunningImageError::Io(error.into())
}
fn verified_dir(dir: &File, mode: u32) -> Result<(), RunningImageError> {
    let meta = dir.metadata()?;
    if !meta.is_dir()
        || meta.uid() != 0
        || meta.nlink() == 0
        || meta.permissions().mode() & 0o7777 != mode
    {
        return Err(RunningImageError::UntrustedImage);
    }
    Ok(())
}
fn trusted_executable(meta: &Metadata, expected: &BootstrapHelperHold) -> bool {
    meta.is_file()
        && meta.uid() == 0
        && meta.nlink() == 1
        && meta.permissions().mode() & 0o7777 == EXEC_MODE
        && meta.len() >= 64
        && meta.len() <= MAX_HELPER_BYTES
        && meta.len() == expected.size
}
fn same_inode(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}
fn unchanged(a: &Metadata, b: &Metadata) -> bool {
    same_inode(a, b)
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
        && a.permissions().mode() == b.permissions().mode()
}

/// Private FD primitive: positive tests use two independently opened handles
/// to ONE disposable root-owned image; the shipping entrypoint has no FD
/// arguments and obtains the second FD ONLY from kernel /proc/self/exe.
fn verify_identical_images(
    fixed: &File,
    running: &File,
    expected: &BootstrapHelperHold,
) -> Result<(), RunningImageError> {
    let on_disk = fixed.metadata()?;
    let self_image = running.metadata()?;
    if !trusted_executable(&on_disk, expected)
        || !trusted_executable(&self_image, expected)
        || !same_inode(&on_disk, &self_image)
        || !expected.remains_hold
        || expected.sha256 == [0; 32]
    {
        return Err(RunningImageError::UntrustedImage);
    }
    let mut ctx = Context::new(&SHA256);
    let mut first = [0u8; 20];
    let mut offset = 0u64;
    let mut buf = [0u8; 32 * 1024];
    while offset < expected.size {
        let want = buf.len().min((expected.size - offset) as usize);
        let n = running.read_at(&mut buf[..want], offset)?;
        if n == 0 {
            return Err(RunningImageError::UntrustedImage);
        }
        if offset < first.len() as u64 {
            let count = (first.len() - offset as usize).min(n);
            first[offset as usize..offset as usize + count].copy_from_slice(&buf[..count]);
        }
        ctx.update(&buf[..n]);
        offset += n as u64;
    }
    let mut extra = [0u8; 1];
    if running.read_at(&mut extra, expected.size)? != 0
        || first[..7] != b"\x7fELF\x02\x01\x01"[..]
        || first[18..20] != b"\x3e\x00"[..]
        || ctx.finish().as_ref() != expected.sha256.as_slice()
        || !unchanged(&on_disk, &fixed.metadata()?)
        || !unchanged(&self_image, &running.metadata()?)
    {
        return Err(RunningImageError::UntrustedImage);
    }
    Ok(())
}

/// This is a guard for an ALREADY running process, not an executor. The fixed
/// root-only installation path is never accepted from the caller or SSH.
/// The signed first-root data staging remains separate and mode 0400; the
/// installed executable must be 0500 and exactly the same authenticated bytes.
pub(super) fn verify_installed_self_image(
    expected: &BootstrapHelperHold,
) -> Result<(), RunningImageError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RunningImageError::NotRoot);
    }
    let mut dir = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    verified_dir(&dir, 0o755)?;
    for (name, mode) in [("usr", 0o755), ("libexec", 0o755), ("akula", 0o700)] {
        dir = File::from(
            openat(
                &dir,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(os)?,
        );
        verified_dir(&dir, mode)?;
    }
    let installed = File::from(
        openat(
            &dir,
            EXEC_LEAF,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    // /proc/self/exe is a kernel-managed magic link to THIS process's
    // executable inode, including the '(deleted)' case. The exact inode
    // equality with the independently opened fixed file refuses deleted,
    // relocated and unrelated root binaries, even with identical contents.
    let running = File::open("/proc/self/exe")?;
    verify_identical_images(&installed, &running, expected)
}

#[cfg(test)]
mod tests;
