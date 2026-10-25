//! Owner-operated B1d0 witness genesis on a clean VPS.
//!
//! Invoked ONLY by an independently trusted and already elevated root-admin
//! binary. No SSH, IPC, environment paths or user-controlled keys. This is
//! NOT Node installation or network access, and does NOT enable systemd.
//! Any prior /var/lib/akula state or crash partial requires manual review.
use super::*;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustix::fs::{flock, mkdirat, renameat_with, FlockOperation, RenameFlags};
use std::io::Write;
use zeroize::Zeroize;

const PROVISIONING_ROOT: &str = "akula";
const BOOTSTRAP: &str = "server-bootstrap";
const TRUST: &str = "trust";
const JOURNAL: &str = "journal";
const SEED: &str = "witness.seed";
const PUB: &str = "witness.pub";
const MARKER: &str = "enrollment";

#[derive(Debug)]
pub enum WitnessGenesisError {
    NotRoot,
    UntrustedParent,
    UnsupportedHost,
    ExistingOrInterrupted,
    Entropy,
    InvalidWitness,
    Io(std::io::Error),
}
impl From<std::io::Error> for WitnessGenesisError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
fn syscall(err: rustix::io::Errno) -> WitnessGenesisError {
    WitnessGenesisError::Io(err.into())
}
fn new_dir(parent: &File, leaf: &str) -> Result<File, WitnessGenesisError> {
    // EXCLUSIVE mkdir. An existing empty directory, dangling symlink or
    // remnant of a crashed attempt is a HARD refusal, never a retry.
    mkdirat(parent, leaf, Mode::RUSR | Mode::WUSR | Mode::XUSR)
        .map_err(|_| WitnessGenesisError::ExistingOrInterrupted)?;
    parent.sync_all()?;
    let dir = File::from(
        openat(
            parent,
            leaf,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&dir.metadata()?, 0o700) {
        return Err(WitnessGenesisError::UntrustedParent);
    }
    Ok(dir)
}
fn write_new(parent: &File, leaf: &str, bytes: &[u8]) -> Result<(), WitnessGenesisError> {
    let pending = format!("{leaf}.pending");
    let mut fd = File::from(
        openat(
            parent,
            pending.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| WitnessGenesisError::ExistingOrInterrupted)?,
    );
    if !root_file_ok(&fd.metadata()?, 0o600, 128) {
        return Err(WitnessGenesisError::UntrustedParent);
    }
    fd.write_all(bytes)?;
    fd.sync_all()?;
    drop(fd);
    renameat_with(
        parent,
        pending.as_str(),
        parent,
        leaf,
        RenameFlags::NOREPLACE,
    )
    .map_err(|_| WitnessGenesisError::ExistingOrInterrupted)?;
    parent.sync_all()?;
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Never,
    AfterDirectories,
    AfterSeed,
    AfterPublic,
}

/// The only test-injectable part accepts a directory FD *already* controlled
/// by root; production's path selection is fixed outside this function.
fn initialize_once(lib: &File, stop: Stop) -> Result<(), WitnessGenesisError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(WitnessGenesisError::NotRoot);
    }
    if !root_dir_ok(&lib.metadata()?, 0o755) {
        return Err(WitnessGenesisError::UntrustedParent);
    }
    flock(lib, FlockOperation::LockExclusive).map_err(syscall)?;
    let result = (|| {
        let root = new_dir(lib, PROVISIONING_ROOT)?;
        let bootstrap = new_dir(&root, BOOTSTRAP)?;
        let trust = new_dir(&bootstrap, TRUST)?;
        let journal = new_dir(&bootstrap, JOURNAL)?;
        if stop == Stop::AfterDirectories {
            return Err(WitnessGenesisError::ExistingOrInterrupted);
        }
        // Entropy comes from the host OS, NEVER a client-provided seed or CI
        // fixture. The private seed is stored only on a 0600 UID0 inode.
        let mut seed = [0u8; 32];
        SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| WitnessGenesisError::Entropy)?;
        if seed == [0; 32] {
            seed.zeroize();
            return Err(WitnessGenesisError::Entropy);
        }
        let pair = Ed25519KeyPair::from_seed_unchecked(&seed);
        let public = pair
            .as_ref()
            .map_err(|_| WitnessGenesisError::Entropy)?
            .public_key()
            .as_ref()
            .to_vec();
        let stored = write_new(&trust, SEED, &seed);
        seed.zeroize();
        stored?;
        if stop == Stop::AfterSeed {
            return Err(WitnessGenesisError::ExistingOrInterrupted);
        }
        write_new(&trust, PUB, &public)?;
        if stop == Stop::AfterPublic {
            return Err(WitnessGenesisError::ExistingOrInterrupted);
        }
        // The exact journal directory has just been created exclusively;
        // no prior or injected CAS revision can be silently adopted.
        if !root_dir_ok(&journal.metadata()?, 0o700) {
            return Err(WitnessGenesisError::InvalidWitness);
        }
        // Reopen both root-owned files and independently rederive the key
        // before the final marker is allowed to exist.
        let mut seed_readback =
            read_root_file(&trust, SEED, 32).map_err(|_| WitnessGenesisError::InvalidWitness)?;
        let matches_seed = Ed25519KeyPair::from_seed_unchecked(&seed_readback)
            .map(|pair| pair.public_key().as_ref() == public.as_slice())
            .unwrap_or(false);
        seed_readback.zeroize();
        if !matches_seed
            || read_root_file(&trust, PUB, 32).map_err(|_| WitnessGenesisError::InvalidWitness)?
                != public
        {
            return Err(WitnessGenesisError::InvalidWitness);
        }
        // Marker last: no interrupted key or directory is ever accepted as
        // an authorized signing witness by OsWitnessLab.
        write_new(&trust, MARKER, WITNESS_ENROLLMENT)?;
        if read_root_file(&trust, MARKER, 128).map_err(|_| WitnessGenesisError::InvalidWitness)?
            != WITNESS_ENROLLMENT
        {
            return Err(WitnessGenesisError::InvalidWitness);
        }
        // The journal is empty and the enrollment marker now exists;
        // OsWitnessLab must separately inspect the exact protected paths.
        Ok(())
    })();
    let unlock = flock(lib, FlockOperation::Unlock).map_err(syscall);
    unlock?;
    result
}

/// Fixed /var/lib parent; this function neither accepts arbitrary paths
/// nor can be activated by any remote Windows/SSH operation.
pub fn admin_initialize_first_root_witness() -> Result<(), WitnessGenesisError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(WitnessGenesisError::NotRoot);
    }
    // Refuse before creating /var/lib/akula on unsupported hosts,
    // including containers without an active systemd manager.
    observed_root_host().map_err(|_| WitnessGenesisError::UnsupportedHost)?;
    let slash = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&slash.metadata()?, 0o755) {
        return Err(WitnessGenesisError::UntrustedParent);
    }
    let var = File::from(
        openat(
            &slash,
            "var",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&var.metadata()?, 0o755) {
        return Err(WitnessGenesisError::UntrustedParent);
    }
    let lib = File::from(
        openat(
            &var,
            "lib",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    initialize_once(&lib, Stop::Never)
}

#[cfg(test)]
mod tests;
