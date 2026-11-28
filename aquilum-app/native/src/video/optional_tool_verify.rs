//! Shared exact-size and SHA-256 admission for pinned optional tools.
use super::{check_ancestors, checked_path};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub(super) fn verify(
    path: &Path, bytes: u64, sha256: &str,
    [parent, length, grew, hash, changed]: [&str; 5],
) -> Result<(), String> {
    check_ancestors(path.parent().ok_or(parent)?)?;
    checked_path(path, false)?;
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() != bytes {
        return Err(length.into());
    }
    let mut digest = Sha256::new();
    let mut read_total = 0u64;
    let mut buffer = [0u8; 65_536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        read_total += n as u64;
        if read_total > bytes { return Err(grew.into()); }
        digest.update(&buffer[..n]);
    }
    let hex = digest.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>();
    if read_total != bytes || hex != sha256 {
        return Err(hash.into());
    }
    checked_path(path, false)?;
    if file.metadata().map_err(|e| e.to_string())?.len() != bytes {
        return Err(changed.into());
    }
    Ok(())
}
