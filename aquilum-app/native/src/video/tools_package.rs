//! VIDEO-TOOLS-01b: read-only, non-shipping signed metadata and byte admission.
//! This does not permit launching tools or constitute a publisher-signed release.
//! Never use an environment-selected key, directory, or executable as authority.
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub(super) const MAX_MANIFEST: usize = 32 * 1024;
pub(super) const MAX_SIGNATURE: usize = 16 * 1024;
const MAX_COMPONENT: u64 = 128 * 1024 * 1024;
const NAMES: [&str; 3] = [
    "akula-video-mux.exe", "akula-yt-dlp.exe", "akula-quickjs.exe",
];
// A Video Tools publisher root has NOT been separately approved.
// Do not silently inherit the stock Akula update signing key.
pub(super) const APPROVED_VIDEO_TOOLS_KEY: &str = "";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    product: String,
    version: String,
    platform: String,
    source_commit: String,
    cargo_lock_sha256: String,
    binaries: Vec<Component>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    name: String,
    sha256: String,
    size: u64,
}
#[derive(Debug)]
pub(super) struct AttestedContents {
    version: String,
    digests: [String; 3],
}

fn hold(message: &str) -> String { format!("VIDEO_TOOLS_01B_HOLD: {message}") }
fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn version(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|part| {
        !part.is_empty() && part.len() <= 10
        && !(part.len() > 1 && part.starts_with('0'))
        && part.bytes().all(|b| b.is_ascii_digit())
        && part.parse::<u32>().is_ok()
    }) { return false; }
    let value: Vec<u32> = parts.iter().map(|part| part.parse().unwrap()).collect();
    value.as_slice() >= [0, 1, 0].as_slice()
}
fn regular(path: &Path, dir: bool) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|_| hold("package path is unavailable"))?;
    if meta.file_type().is_symlink()
        || (dir && !meta.is_dir())
        || (!dir && !meta.is_file())
    { return Err(hold("package path is not a regular file or directory")); }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(hold("package path is a Windows reparse point"));
        }
    }
    Ok(())
}
pub(super) fn bounded_metadata(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    regular(path, false)?;
    let file = File::open(path).map_err(|_| hold("cannot read package metadata"))?;
    let mut bounded = file.take(limit as u64 + 1);
    let mut bytes = Vec::new();
    bounded.read_to_end(&mut bytes).map_err(|_| hold("cannot read package metadata"))?;
    if bytes.is_empty() || bytes.len() > limit {
        return Err(hold("installed metadata empty or oversized"));
    }
    regular(path, false)?;
    Ok(bytes)
}
fn hash_file(path: &Path, size: u64, expected: &str) -> Result<(), String> {
    regular(path, false)?;
    let mut file = File::open(path).map_err(|_| hold("cannot open component"))?;
    let before = file.metadata().map_err(|_| hold("cannot stat component"))?;
    if before.len() != size || size < 64 || size > MAX_COMPONENT {
        return Err(hold("component size is incorrect"));
    }
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    let mut count = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|_| hold("component read failed"))?;
        if n == 0 { break; }
        count = count.checked_add(n as u64).ok_or_else(|| hold("size overflow"))?;
        if count > size { return Err(hold("component grew during hash")); }
        hash.update(&buf[..n]);
    }
    let after = file.metadata().map_err(|_| hold("component vanished"))?;
    regular(path, false)?;
    if count != size || after.len() != size {
        return Err(hold("component changed during hash"));
    }
    let digest: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != expected { return Err(hold("component hash differs from signed manifest")); }
    Ok(())
}
// This accepts ONLY manifest bytes already authenticated by Minisign.
fn inspect_authenticated(root: &Path, raw: &[u8]) -> Result<AttestedContents, String> {
    if !root.is_absolute() { return Err(hold("package root must be absolute")); }
    regular(root, true)?;
    let data: Manifest = serde_json::from_slice(raw)
        .map_err(|_| hold("malformed, ambiguous or unknown-field signed manifest"))?;
    if data.schema != 1 || data.product != "Akula Video Tools"
        || data.platform != "windows-x86_64" || !version(&data.version)
        || !hex(&data.source_commit, 40) || !hex(&data.cargo_lock_sha256, 64)
        || data.binaries.len() != 3
    { return Err(hold("wrong signed product, source, version or three-PE identity")); }
    let mut digests = [String::new(), String::new(), String::new()];
    for (i, entry) in data.binaries.iter().enumerate() {
        if entry.name != NAMES[i] || !hex(&entry.sha256, 64)
            || !(64..=MAX_COMPONENT).contains(&entry.size)
        { return Err(hold("wrong signed component name, digest or size")); }
        hash_file(&root.join(NAMES[i]), entry.size, &entry.sha256)?;
        digests[i] = entry.sha256.clone();
    }
    // This read-only witness tolerates only exact known package files, not
    // a second Akula app, arbitrary EXE, nested directory or user-data path.
    let allowed = [
        NAMES[0], NAMES[1], NAMES[2],
        "manifest.json", "manifest.minisig", "uninstall.exe",
    ];
    let mut entry_count = 0usize;
    for entry in fs::read_dir(root).map_err(|_| hold("cannot enumerate package"))? {
        entry_count += 1;
        if entry_count > allowed.len() { return Err(hold("too many package entries")); }
        let entry = entry.map_err(|_| hold("cannot enumerate package item"))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| hold("non-UTF8 package entry"))?;
        if !allowed.contains(&name) { return Err(hold("unknown file in protected tools root")); }
        regular(&entry.path(), false)?;
    }
    regular(root, true)?;
    Ok(AttestedContents { version: data.version, digests })
}

pub(super) fn verify_signed(root: &Path, raw: &[u8], signed: &str, trusted_key: &str)
    -> Result<AttestedContents, String>
{
    if trusted_key.is_empty() { return Err(hold("publisher signing key not approved")); }
    if raw.is_empty() || raw.len() > MAX_MANIFEST
        || signed.is_empty() || signed.len() > MAX_SIGNATURE
    { return Err(hold("unsigned, empty or oversized metadata")); }
    let key = PublicKey::decode(trusted_key).map_err(|_| hold("publisher key invalid"))?;
    let signature = Signature::decode(signed).map_err(|_| hold("signature malformed"))?;
    key.verify(raw, &signature, true)
        .map_err(|_| hold("publisher signature failed"))?;
    // Authenticate the actual installed manifest, not caller-selected bytes
    // from another directory. Never follow a symlink or reparse alias.
    let manifest_path = root.join("manifest.json");
    let signature_path = root.join("manifest.minisig");
    let disk_manifest = bounded_metadata(&manifest_path, MAX_MANIFEST)?;
    let disk_signature = bounded_metadata(&signature_path, MAX_SIGNATURE)?;
    if disk_manifest != raw || disk_signature.as_slice() != signed.as_bytes() {
        return Err(hold("disk metadata differs from verified signed bytes"));
    }
    inspect_authenticated(root, raw)
}

// No shipping discovery/launch is enabled by simply merging this module.
// A separate owner-key, protected-root ACL and actual signed install gate
// must be accepted before any caller can use these file digests for execution.
pub(super) fn shipping_admission_enabled() -> bool {
    false // a future, separately reviewed release gate must replace this constant
}

#[cfg(test)]
#[path = "tools_package_tests.rs"]
mod tests;
