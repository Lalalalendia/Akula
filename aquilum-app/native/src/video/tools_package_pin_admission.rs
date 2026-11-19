//! VIDEO-TOOLS-01g: diagnostic Minisign-to-held-Windows-file binding.
//! Never launches tools, returns executable paths or authorizes shipping.
#![allow(dead_code)]
use std::io::Read;
use std::path::Path;
use sha2::{Digest, Sha256};
use super::tools_package::{
    verify_signed, shipping_admission_enabled, APPROVED_VIDEO_TOOLS_KEY,
    MAX_MANIFEST, MAX_SIGNATURE,
};
use super::tools_package_probe::windows_discovery;
#[path = "tools_package_pin_windows.rs"]
mod windows_pin;

pub(super) struct ReadOnlyPinnedAttestation {
    _handles: windows_pin::PinnedPackage,
    _signed_version: String,
}
fn hold(message: &str) -> String {
    format!("VIDEO_TOOLS_01G_HOLD: {message}")
}
/// Verifies signed manifest and all three PE digests while holding SAME
/// read-only handles with no FILE_SHARE_WRITE or FILE_SHARE_DELETE.
/// The guard lifetime is kept in the returned diagnostic-only structure.
pub(super) fn inspect_owner_approved_package(
    root: &Path, manifest: &[u8], signed: &str,
) -> Result<ReadOnlyPinnedAttestation, String> {
    if APPROVED_VIDEO_TOOLS_KEY.is_empty() || !shipping_admission_enabled() {
        return Err(hold("Video Tools publisher key or release approval absent"));
    }
    let expected = windows_discovery::installed_root()?
        .ok_or_else(|| hold("protected package not installed"))?;
    if expected != root { return Err(hold("caller-selected package root refused")); }
    windows_discovery::check_installed_acl(root, &windows_pin::SIGNED_NAMES)?;
    let mut held = windows_pin::PinnedPackage::open(root, &windows_pin::SIGNED_NAMES)?;
    let attested = verify_signed(root, manifest, signed, APPROVED_VIDEO_TOOLS_KEY)?;
    held.inspect(|name, file| {
        if name == "manifest.json" || name == "manifest.minisig" {
            let (expected, maximum): (&[u8], usize) = if name == "manifest.json" {
                (manifest, MAX_MANIFEST)
            } else { (signed.as_bytes(), MAX_SIGNATURE) };
            let mut raw = Vec::new();
            file.take(maximum as u64 + 1).read_to_end(&mut raw)
                .map_err(|_| hold("held metadata read failed"))?;
            if raw.len() > maximum || raw != expected {
                return Err(hold("held metadata differs from signed bytes"));
            }
        } else if let Some(i) = [
            "akula-video-mux.exe", "akula-yt-dlp.exe", "akula-quickjs.exe",
        ].iter().position(|item| *item == name) {
            let mut digest = Sha256::new();
            let mut buf = [0u8; 65536];
            let mut size = 0u64;
            loop {
                let n = file.read(&mut buf).map_err(|_| hold("held PE unreadable"))?;
                if n == 0 { break; }
                size = size.checked_add(n as u64).ok_or_else(|| hold("PE length overflow"))?;
                if size > 128 * 1024 * 1024 { return Err(hold("held PE oversized")); }
                digest.update(&buf[..n]);
            }
            let hash: String = digest.finalize().iter().map(|v| format!("{v:02x}")).collect();
            if hash != attested.digests[i] {
                return Err(hold("held PE digest differs from authenticated manifest"));
            }
        }
        Ok(())
    })?;
    Ok(ReadOnlyPinnedAttestation { _handles: held, _signed_version: attested.version })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_owner_key_never_grants_handle_admission() {
        assert!(inspect_owner_approved_package(
            Path::new("C:\\Program Files\\Akula Video Tools"), b"{}", "untrusted"
        ).is_err());
        assert!(APPROVED_VIDEO_TOOLS_KEY.is_empty());
        assert!(!shipping_admission_enabled());
    }
}
