//! Durable Iroh transport identity, separate from the Node Ed25519 receipt key.
//! The file is confined to the dedicated 0700 non-root StateDirectory.

use iroh::SecretKey;
use ring::rand::{SecureRandom, SystemRandom};
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const KEY: &str = "node-iroh-seed.bin";

pub(super) fn load_or_create(state: &Path) -> Result<SecretKey, &'static str> {
    let before = fs::symlink_metadata(state).map_err(|_| "state directory unavailable")?;
    let uid = rustix::process::geteuid().as_raw();
    if !before.is_dir() || before.uid() != uid
        || before.permissions().mode() & 0o7777 != 0o700
    {
        return Err("unsafe Iroh key directory");
    }
    let dir = File::from(open(
        state,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(|_| "untrusted Iroh key directory")?);
    let after = dir.metadata().map_err(|_| "Iroh directory metadata unavailable")?;
    if before.dev() != after.dev() || before.ino() != after.ino() {
        return Err("Iroh key directory replaced");
    }
    let mut handle = match openat(
        &dir, KEY,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => {
            let mut seed = [0u8; 32];
            SystemRandom::new().fill(&mut seed)
                .map_err(|_| "Iroh identity entropy unavailable")?;
            let mut fresh = File::from(openat(
                &dir, KEY,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL
                    | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ).map_err(|_| "Iroh key creation failed")?);
            fresh.write_all(&seed).map_err(|_| "Iroh key write failed")?;
            fresh.sync_all().map_err(|_| "Iroh key sync failed")?;
            dir.sync_all().map_err(|_| "Iroh directory sync failed")?;
            drop(fresh);
            File::from(openat(
                &dir, KEY,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ).map_err(|_| "Iroh key reopen failed")?)
        }
        Err(_) => return Err("Iroh identity file refused"),
    };
    let info = handle.metadata().map_err(|_| "Iroh key metadata failed")?;
    if !info.is_file() || info.uid() != uid || info.nlink() != 1
        || info.len() != 32 || info.permissions().mode() & 0o7777 != 0o600
    {
        return Err("unsafe Iroh private key");
    }
    let mut seed = [0u8; 32];
    handle.read_exact(&mut seed).map_err(|_| "corrupt Iroh identity")?;
    Ok(SecretKey::from_bytes(&seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_iroh_peer_survives_cold_reopen_and_denies_bad_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let id = load_or_create(&state).unwrap().public();
        assert_eq!(load_or_create(&state).unwrap().public(), id);
        let path = state.join(KEY);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create(&state).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
        assert!(load_or_create(&state).is_err());
    }
}
