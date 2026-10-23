//! Never rotate Iroh transport identity from a running service.
//! A signed root-approved installer must first enroll this 32-byte seed
//! and pin its exact public EndpointId in owner-policy.json (v3).

use iroh::SecretKey;
use rustix::fs::{Mode, OFlags, open, openat};
use std::fs::{self, File};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const KEY: &str = "node-iroh-seed.bin";

pub(super) fn load_provisioned(state: &Path) -> Result<SecretKey, &'static str> {
    let before = fs::symlink_metadata(state).map_err(|_| "state directory unavailable")?;
    let uid = rustix::process::geteuid().as_raw();
    if !before.is_dir() || before.uid() != uid || before.permissions().mode() & 0o7777 != 0o700 {
        return Err("unsafe Iroh key directory");
    }
    let dir = File::from(
        open(
            state,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| "untrusted Iroh key directory")?,
    );
    let after = dir
        .metadata()
        .map_err(|_| "Iroh directory metadata unavailable")?;
    if before.dev() != after.dev() || before.ino() != after.ino() {
        return Err("Iroh key directory replaced");
    }
    let mut handle = File::from(
        openat(
            &dir,
            KEY,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| "Iroh identity not enrolled or unsafe")?,
    );
    let info = handle.metadata().map_err(|_| "Iroh key metadata failed")?;
    if !info.is_file()
        || info.uid() != uid
        || info.nlink() != 1
        || info.len() != 32
        || info.permissions().mode() & 0o7777 != 0o600
    {
        return Err("unsafe Iroh private key");
    }
    let mut seed = [0u8; 32];
    handle
        .read_exact(&mut seed)
        .map_err(|_| "corrupt Iroh identity")?;
    Ok(SecretKey::from_bytes(&seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    #[test]
    fn installed_key_is_stable_and_missing_or_unsafe_keys_never_regenerate() {
        let tmp = tempfile::tempdir().unwrap();
        let state = tmp.path().join("state");
        fs::create_dir(&state).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        let path = state.join(KEY);
        assert!(load_provisioned(&state).is_err());
        assert!(!path.exists());

        // Only a test fixture simulates a trusted installer provisioning.
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .unwrap();
        file.write_all(&[23; 32]).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let id = load_provisioned(&state).unwrap().public();
        assert_eq!(load_provisioned(&state).unwrap().public(), id);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_provisioned(&state).is_err());
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", &path).unwrap();
        assert!(load_provisioned(&state).is_err());
        fs::remove_file(&path).unwrap();
        assert!(load_provisioned(&state).is_err());
        assert!(!path.exists());
    }
}
