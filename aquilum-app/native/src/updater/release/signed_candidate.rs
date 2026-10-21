//! RELEASE-SIGNED-ADMISSION-01: typed Akula candidate admission.
//!
//! A *signed manifest* must bind version, source, cargo lock, platform,
//! expected filename, SHA-256 and size of the exact candidate bytes.
//! This module cannot publish anything while the Akula publisher trust root
//! is unset and the release channel remains disabled.
//!
//! It is deliberately not yet connected to apply(): future work must also
//! couple the verified candidate to the OS transaction and a startup ACK.

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{PUBLIC_KEY, channel_ready, verify};

const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_LINUX_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const TARGET_LINUX_APPIMAGE: &str = "linux-x86_64-appimage";
const TARGET_WINDOWS_NSIS: &str = "windows-x86_64";
const MAX_WINDOWS_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

fn hold(why: &str) -> String {
    format!("AKULA_SIGNED_CANDIDATE_HOLD: {why}")
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedManifest {
    schema: u8,
    product: String,
    version: String,
    source_commit: String,
    cargo_lock_sha256: String,
    artifacts: PlatformArtifacts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlatformArtifacts {
    // A protected single-OS publisher run may sign only artifacts it built.
    // Never require synthetic digests for another OS. The selected platform
    // is mandatory below; every present entry remains strictly validated.
    #[serde(rename = "windows-x86_64", default)]
    windows: Option<Artifact>,
    #[serde(rename = "linux-x86_64-appimage", default)]
    appimage: Option<Artifact>,
    #[serde(rename = "linux-x86_64-deb", default)]
    deb: Option<Artifact>,
    #[serde(rename = "linux-x86_64-rpm", default)]
    rpm: Option<Artifact>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    filename: String,
    sha256: String,
    size: u64,
}

/// This type has private fields and no public unchecked constructor.
/// Do not replace it with Vec<u8> at the filesystem transaction boundary.
#[derive(Debug)]
pub(crate) struct SignedCandidate {
    bytes: Vec<u8>,
    version: String,
    source_commit: String,
    sha256: String,
}

impl SignedCandidate {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Bytes leave the sealed candidate only for the legacy UI handoff,
    /// whose Windows apply entrypoint still refuses installation. The final
    /// transaction boundary must receive SignedCandidate, never this Vec.
    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub(crate) fn version(&self) -> &str {
        &self.version
    }

    pub(crate) fn source_commit(&self) -> &str {
        &self.source_commit
    }

    pub(crate) fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Rehash the exact staged Linux inode under the updater's OS lock.
    /// Checking a previously downloaded Vec is not enough: the on-disk copy
    /// itself must match the signed byte digest before any rename.
    #[cfg(target_os = "linux")]
    pub(crate) fn verify_staged_file(&self, path: &std::path::Path) -> Result<(), String> {
        use std::io::Read;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        // Linux O_NOFOLLOW; refuse the final path being replaced with a link.
        const LINUX_O_NOFOLLOW: i32 = 0o400000;
        let mut handle = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(LINUX_O_NOFOLLOW)
            .open(path)
            .map_err(|e| format!("{}: {e}", hold("cannot open nonlinked staged image")))?;
        let before = handle.metadata().map_err(|e| e.to_string())?;
        let entry = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !before.is_file()
            || !entry.file_type().is_file()
            || before.nlink() != 1
            || before.ino() != entry.ino()
            || before.dev() != entry.dev()
            || before.len() != self.bytes.len() as u64
        {
            return Err(hold("staged file authority/type/size mismatch"));
        }
        let mut digest = Sha256::new();
        let mut total = 0u64;
        let mut block = [0u8; 65536];
        loop {
            let read = handle.read(&mut block).map_err(|e| e.to_string())?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64)
                .ok_or_else(|| hold("staged image length overflow"))?;
            if total > self.bytes.len() as u64 {
                return Err(hold("staged image grew while hashing"));
            }
            digest.update(&block[..read]);
        }
        let after = handle.metadata().map_err(|e| e.to_string())?;
        let still_named = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if total != self.bytes.len() as u64
            || after.len() != total
            || after.ino() != before.ino()
            || after.dev() != before.dev()
            || still_named.ino() != before.ino()
            || still_named.dev() != before.dev()
            || !still_named.file_type().is_file()
        {
            return Err(hold("staged inode changed or was replaced during hashing"));
        }
        let actual: String = digest.finalize().iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if actual != self.sha256 {
            return Err(hold("staged disk bytes differ from signed publisher hash"));
        }
        Ok(())
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_hex(value: &str, count: usize) -> bool {
    value.len() == count
        && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn numeric_version(value: &str) -> Result<[u32; 3], String> {
    if value.is_empty() || value.len() > 32 {
        return Err(hold("invalid version"));
    }
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3 {
        return Err(hold("only three-part stable versions are approved"));
    }
    let mut version = [0u32; 3];
    for (i, part) in parts.into_iter().enumerate() {
        if part.is_empty()
            || part.len() > 10
            || (part.starts_with('0') && part.len() > 1)
            || !part.bytes().all(|c| c.is_ascii_digit())
        {
            return Err(hold("ambiguous or nonnumeric version"));
        }
        version[i] = part.parse().map_err(|_| hold("version overflow"))?;
    }
    Ok(version)
}

// Every PRESENT signed platform must have usable, bounded, unambiguous
// metadata; absent platforms are not claimed by a single-platform build.
// A client must reject absence of its own required signed artifact.
const MAX_RELEASE_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;

fn validate_auxiliary_asset(
    item: &Artifact,
    expected: &str,
    platform: &str,
) -> Result<(), String> {
    if item.filename != expected
        || !valid_hex(&item.sha256, 64)
        || item.size == 0
        || item.size > MAX_RELEASE_ARTIFACT_BYTES
    {
        return Err(hold(&format!("invalid signed {platform} artifact identity")));
    }
    Ok(())
}

fn validate_windows_asset(item: &Artifact, version: &str) -> Result<(), String> {
    let prefix = format!("Akula_{version}_");
    let variant = item.filename.strip_prefix(&prefix)
        .and_then(|name| name.strip_suffix("-setup.exe"))
        .ok_or_else(|| hold("invalid signed Windows artifact filename"))?;
    if variant.is_empty()
        || !variant.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || !valid_hex(&item.sha256, 64)
        || item.size == 0
        || item.size > MAX_WINDOWS_INSTALLER_BYTES
    {
        return Err(hold("invalid signed Windows artifact identity"));
    }
    Ok(())
}

/// Authenticated offer metadata; not a filesystem write grant.
/// In particular, a Windows NSIS offer is installer_required, never an
/// authorization to run the legacy in-place updater.
#[derive(Clone, Debug)]
pub(super) struct VerifiedOffer {
    pub(super) version: String,
    pub(super) filename: String,
    pub(super) sha256: String,
    pub(super) size: u64,
    pub(super) source_commit: String,
}

/// Parse a manifest only after its exact raw bytes have been authenticated.
/// This structural helper is also used by the byte-level admission tests;
/// it never verifies a signature on its own.
fn inspect_offer_authenticated(
    raw: &[u8],
    expected_platform: &str,
    running_version: &str,
) -> Result<Option<VerifiedOffer>, String> {
    if raw.is_empty() || raw.len() > MAX_MANIFEST_BYTES {
        return Err(hold("signed manifest size limit"));
    }
    if expected_platform != TARGET_LINUX_APPIMAGE && expected_platform != TARGET_WINDOWS_NSIS {
        return Err(hold("unsupported or unexpected platform for automatic apply"));
    }
    let manifest: SignedManifest = serde_json::from_slice(raw)
        .map_err(|_| hold("ambiguous, malformed or unknown-field manifest"))?;
    if manifest.schema != 1 || manifest.product != "Akula" {
        return Err(hold("wrong product or schema"));
    }
    if !valid_hex(&manifest.source_commit, 40)
        || !valid_hex(&manifest.cargo_lock_sha256, 64)
    {
        return Err(hold("invalid source or lock provenance"));
    }

    let next = numeric_version(&manifest.version)?;
    let current = numeric_version(running_version)?;
    if next < current {
        return Err(hold("release is not strictly newer than the installed version"));
    }

    // A signed Windows-only candidate may omit unbuilt Linux artifacts.
    // Nevertheless, reject malformed cross-platform entries if they appear.
    if let Some(item) = &manifest.artifacts.windows {
        validate_windows_asset(item, &manifest.version)?;
    }
    if let Some(item) = &manifest.artifacts.appimage {
        validate_auxiliary_asset(
            item,
            &format!("akula-app_{}_x86_64.AppImage", manifest.version),
            "AppImage",
        )?;
        if item.size > MAX_LINUX_IMAGE_BYTES as u64 {
            return Err(hold("invalid signed AppImage artifact size"));
        }
    }
    if let Some(item) = &manifest.artifacts.deb {
        validate_auxiliary_asset(
            item,
            &format!("Akula_{}_amd64.deb", manifest.version),
            "DEB",
        )?;
    }
    if let Some(item) = &manifest.artifacts.rpm {
        validate_auxiliary_asset(
            item,
            &format!("Akula-{}-1.x86_64.rpm", manifest.version),
            "RPM",
        )?;
    }

    let (asset, expected_name, budget) = if expected_platform == TARGET_WINDOWS_NSIS {
        (
            manifest.artifacts.windows.as_ref()
                .ok_or_else(|| hold("required signed Windows artifact missing"))?,
            format!("Akula_{}_x64-setup.exe", manifest.version),
            MAX_WINDOWS_INSTALLER_BYTES,
        )
    } else {
        (
            manifest.artifacts.appimage.as_ref()
                .ok_or_else(|| hold("required signed AppImage artifact missing"))?,
            format!("akula-app_{}_x86_64.AppImage", manifest.version),
            MAX_LINUX_IMAGE_BYTES as u64,
        )
    };
    if asset.filename != expected_name {
        return Err(hold("signed platform filename does not identify the approved Akula installer"));
    }
    if !valid_hex(&asset.sha256, 64) || asset.size == 0 || asset.size > budget {
        return Err(hold("signed asset hash format, size or budget mismatch"));
    }
    if next == current {
        return Ok(None);
    }
    Ok(Some(VerifiedOffer {
        version: manifest.version.clone(),
        filename: asset.filename.clone(),
        sha256: asset.sha256.clone(),
        size: asset.size,
        source_commit: manifest.source_commit.clone(),
    }))
}

/// Never use this without prior signature verification. Used only after the
/// Minisign gate in admit_with_trusted_key or within synthetic unit fixtures.
fn inspect_authenticated(
    raw: &[u8],
    bytes: &[u8],
    expected_platform: &str,
    running_version: &str,
) -> Result<(String, String, String), String> {
    let offer = inspect_offer_authenticated(raw, expected_platform, running_version)?
        .ok_or_else(|| hold("release is not strictly newer than the installed version"))?;
    if offer.size != bytes.len() as u64 {
        return Err(hold("signed asset hash format, size or budget mismatch"));
    }
    if digest_hex(bytes) != offer.sha256 {
        return Err(hold("downloaded bytes are not the signed platform asset"));
    }
    Ok((offer.version, offer.source_commit, offer.sha256))
}

/// The sole positive metadata-checking routine (the key is injectable ONLY
/// within this private module, for real ephemeral Minisign CI fixtures).
fn signed_offer_with_trusted_key(
    manifest: &[u8],
    signature: &str,
    key: &str,
    platform: &str,
    current: &str,
) -> Result<Option<VerifiedOffer>, String> {
    if key.is_empty() {
        return Err(hold("Akula publisher key is not installed"));
    }
    verify(manifest, signature, key)
        .map_err(|_| hold("signed manifest publisher authentication failed"))?;
    inspect_offer_authenticated(manifest, platform, current)
}

/// A production metadata offer has neither a caller-provided trust key nor a
/// caller-provided platform/version. The release channel remains HOLD.
pub(super) fn offer(manifest: &[u8], signature: &str) -> Result<Option<VerifiedOffer>, String> {
    if !channel_ready() || PUBLIC_KEY.is_empty() {
        return Err(hold("Akula signed-release channel remains disabled"));
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        signed_offer_with_trusted_key(
            manifest, signature, PUBLIC_KEY,
            TARGET_WINDOWS_NSIS, env!("CARGO_PKG_VERSION"),
        )
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        signed_offer_with_trusted_key(
            manifest, signature, PUBLIC_KEY,
            TARGET_LINUX_APPIMAGE, env!("CARGO_PKG_VERSION"),
        )
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
    )))]
    {
        let _ = (manifest, signature);
        Err(hold("no signed updater format for this OS/architecture"))
    }
}

fn admit_with_trusted_key(
    manifest: &[u8],
    manifest_signature: &str,
    payload: Vec<u8>,
    key: &str,
    platform: &str,
    current: &str,
) -> Result<SignedCandidate, String> {
    if key.is_empty() {
        return Err(hold("Akula publisher key is not installed"));
    }
    // Verify the exact downloaded manifest bytes BEFORE trusting even its
    // version or URLs. A signature on package bytes alone cannot authenticate
    // platform/version metadata.
    verify(manifest, manifest_signature, key)
        .map_err(|_| hold("signed manifest publisher authentication failed"))?;
    let (version, source_commit, sha256) =
        inspect_authenticated(manifest, &payload, platform, current)?;
    Ok(SignedCandidate { bytes: payload, version, source_commit, sha256 })
}

/// The only production admission entrypoint. No caller-supplied public key,
/// platform or version floor is accepted.
pub(super) fn admit(
    manifest: &[u8],
    signature: &str,
    payload: Vec<u8>,
) -> Result<SignedCandidate, String> {
    if !channel_ready() || PUBLIC_KEY.is_empty() {
        return Err(hold("Akula signed-release channel remains disabled"));
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        admit_with_trusted_key(
            manifest, signature, payload, PUBLIC_KEY,
            TARGET_LINUX_APPIMAGE, env!("CARGO_PKG_VERSION"),
        )
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        admit_with_trusted_key(
            manifest, signature, payload, PUBLIC_KEY,
            TARGET_WINDOWS_NSIS, env!("CARGO_PKG_VERSION"),
        )
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        let _ = (manifest, signature, payload);
        Err(hold("only Windows NSIS or Linux x86_64 AppImage admission is supported"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(data: &[u8], version: &str) -> Vec<u8> {
        let sha = digest_hex(data);
        serde_json::to_vec(&json!({
            "schema":1,
            "product":"Akula",
            "version":version,
            "source_commit":"a".repeat(40),
            "cargo_lock_sha256":"b".repeat(64),
            "artifacts":{
                "windows-x86_64":{"filename":"Akula_0.3.10_x64-setup.exe","sha256":"c".repeat(64),"size":12},
                "linux-x86_64-appimage":{
                    "filename":format!("akula-app_{version}_x86_64.AppImage"),
                    "sha256":sha,"size":data.len()
                },
                "linux-x86_64-deb":{"filename":"Akula_0.3.10_amd64.deb","sha256":"c".repeat(64),"size":12},
                "linux-x86_64-rpm":{"filename":"Akula-0.3.10-1.x86_64.rpm","sha256":"c".repeat(64),"size":12}
            }
        })).unwrap()
    }

    #[test]
    fn exact_structural_witness_accepts_only_matching_signed_shape_and_bytes() {
        let bytes = b"fixture package bytes";
        let raw = fixture(bytes, "0.3.10");
        let (version, source, sha) =
            inspect_authenticated(&raw, bytes, TARGET_LINUX_APPIMAGE, "0.3.9").unwrap();
        assert_eq!(version, "0.3.10");
        assert_eq!(source, "a".repeat(40));
        assert_eq!(sha, digest_hex(bytes));
    }

    #[test]
    fn windows_nsis_requires_exact_publisher_signed_package_identity() {
        // Structural checks here are deliberately not a signature verifier.
        // admit_with_trusted_key authenticates the entire manifest first.
        let package = b"synthetic Windows installer bytes";
        let mut entry: serde_json::Value =
            serde_json::from_slice(&fixture(b"other OS", "0.3.10")).unwrap();
        entry["artifacts"]["windows-x86_64"] = json!({
            "filename": "Akula_0.3.10_x64-setup.exe",
            "sha256": digest_hex(package),
            "size": package.len(),
        });
        let signed_shape = serde_json::to_vec(&entry).unwrap();
        let identity = inspect_authenticated(
            &signed_shape, package, TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap();
        assert_eq!(identity.0, "0.3.10");
        assert_eq!(identity.2, digest_hex(package));

        let mut corrupted = package.to_vec();
        corrupted[0] ^= 1;
        assert!(inspect_authenticated(
            &signed_shape, &corrupted, TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap_err().contains("downloaded bytes"));

        for wrong_version in ["0.3.10", "0.3.11"] {
            assert!(inspect_authenticated(
                &signed_shape, package, TARGET_WINDOWS_NSIS, wrong_version,
            ).unwrap_err().contains("not strictly newer"));
        }
        for wrong_name in [
            "Aquilum_0.3.10_x64-setup.exe",
            "Akula_0.3.10_arm64-setup.exe",
            "Akula_0.3.9_x64-setup.exe",
            "../Akula_0.3.10_x64-setup.exe",
        ] {
            let mut replaced = entry.clone();
            replaced["artifacts"]["windows-x86_64"]["filename"] = json!(wrong_name);
            assert!(inspect_authenticated(
                &serde_json::to_vec(&replaced).unwrap(),
                package, TARGET_WINDOWS_NSIS, "0.3.9",
            ).is_err(), "{wrong_name}");
        }
        assert!(inspect_authenticated(
            &signed_shape, package, TARGET_LINUX_APPIMAGE, "0.3.9",
        ).is_err(), "the same signed entry must not authorize a different platform");
    }

    #[test]
    fn windows_only_signed_manifest_shape_matches_protected_candidate_workflow() {
        let payload = b"real Windows NSIS bytes synthetic fixture";
        let version = "0.3.10";
        let manifest = serde_json::to_vec(&json!({
            "schema": 1,
            "product": "Akula",
            "version": version,
            "source_commit": "a".repeat(40),
            "cargo_lock_sha256": "b".repeat(64),
            "artifacts": {
                "windows-x86_64": {
                    "filename": format!("Akula_{version}_x64-setup.exe"),
                    "sha256": digest_hex(payload),
                    "size": payload.len()
                }
            }
        })).unwrap();
        let accepted = inspect_authenticated(
            &manifest, payload, TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap();
        assert_eq!(accepted.0, version);
        assert_eq!(accepted.2, digest_hex(payload));

        // A Windows-only signed manifest MUST NOT authorize Linux:
        // absence is semantically different from a zero-byte package.
        let missing = inspect_authenticated(
            &manifest, payload, TARGET_LINUX_APPIMAGE, "0.3.9",
        ).unwrap_err();
        assert!(missing.contains("required signed AppImage artifact missing"), "{missing}");

        let mut changed: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
        changed["artifacts"]["linux-x86_64-appimage"] = json!({
            "filename": "Aquilum_0.3.10_x86_64.AppImage",
            "sha256": digest_hex(payload),
            "size": payload.len()
        });
        assert!(inspect_authenticated(
            &serde_json::to_vec(&changed).unwrap(), payload, TARGET_WINDOWS_NSIS, "0.3.9",
        ).is_err(), "a malformed other-OS artifact must still be rejected");
    }

    #[test]
    fn signed_offer_metadata_refuses_rollback_missing_platform_and_unknown_fields() {
        let data = b"synthetic bytes";
        let raw = fixture(data, "0.3.10");
        let offer = inspect_offer_authenticated(&raw, TARGET_LINUX_APPIMAGE, "0.3.9")
            .unwrap().expect("newer authenticated offer");
        assert_eq!(offer.version, "0.3.10");
        assert_eq!(offer.size, data.len() as u64);
        assert_eq!(offer.sha256, digest_hex(data));

        // Equal is not a new offer; an older signed manifest is a HOLD.
        assert!(inspect_offer_authenticated(&raw, TARGET_LINUX_APPIMAGE, "0.3.10")
            .unwrap().is_none());
        assert!(inspect_offer_authenticated(&raw, TARGET_LINUX_APPIMAGE, "0.3.11")
            .unwrap_err().contains("not strictly newer"));
        let mut changed: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        changed["artifacts"].as_object_mut().unwrap()
            .remove("linux-x86_64-appimage");
        assert!(inspect_offer_authenticated(
            &serde_json::to_vec(&changed).unwrap(), TARGET_LINUX_APPIMAGE, "0.3.9",
        ).unwrap_err().contains("required signed AppImage artifact missing"));
        let mut changed: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        changed["untrusted_url"] = json!("https://other.example/update.exe");
        assert!(inspect_offer_authenticated(
            &serde_json::to_vec(&changed).unwrap(), TARGET_LINUX_APPIMAGE, "0.3.9",
        ).is_err());
    }

    #[test]
    fn changed_same_size_bytes_are_rejected() {
        let data = b"fixture package bytes";
        let mut tampered = data.to_vec();
        tampered[0] ^= 1;
        let raw = fixture(data, "0.3.10");
        assert!(inspect_authenticated(&raw, &tampered, TARGET_LINUX_APPIMAGE, "0.3.9")
            .unwrap_err().contains("downloaded bytes"));
    }

    #[test]
    fn product_platform_version_and_downgrade_require_strict_identity() {
        let bytes = b"new";
        let raw = fixture(bytes, "0.3.10");
        assert!(inspect_authenticated(&raw, bytes, "windows-x86_64", "0.3.9").is_err());
        assert!(inspect_authenticated(&raw, bytes, TARGET_LINUX_APPIMAGE, "0.3.10").is_err());
        assert!(inspect_authenticated(&raw, bytes, TARGET_LINUX_APPIMAGE, "0.3.11").is_err());
        for version in ["0.3.9-rc.1", "0.03.10", "0.3", "0.3.0.1", "999999999999.0.0"] {
            assert!(numeric_version(version).is_err(), "{version}");
        }
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        doc["product"] = json!("Aquilum");
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
    }

    #[test]
    fn unknown_fields_and_missing_selected_platform_refuse_admission() {
        let bytes = b"new";
        let raw = fixture(bytes, "0.3.10");
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        // An unused platform may be omitted by a single-OS publisher.
        doc["artifacts"].as_object_mut().unwrap().remove("linux-x86_64-rpm");
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_ok());
        // But omitting the selected AppImage must fail closed.
        doc["artifacts"].as_object_mut().unwrap().remove("linux-x86_64-appimage");
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9")
            .unwrap_err().contains("required signed AppImage artifact missing"));
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        doc["artifacts"]["other-os"] = json!({"filename":"other","sha256":"c".repeat(64),"size":12});
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
        let repeated = String::from_utf8(raw).unwrap().replace("\"product\":\"Akula\"",
            "\"product\":\"Akula\",\"product\":\"Aquilum\"");
        assert!(inspect_authenticated(repeated.as_bytes(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
    }

    #[test]
    fn malformed_other_platform_cannot_hide_inside_signed_linux_candidate() {
        let bytes = b"new";
        let raw = fixture(bytes, "0.3.10");
        for platform in ["windows-x86_64", "linux-x86_64-deb", "linux-x86_64-rpm"] {
            let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            doc["artifacts"][platform]["sha256"] = json!("not-an-actual-sha256");
            assert!(
                inspect_authenticated(
                    &serde_json::to_vec(&doc).unwrap(), bytes,
                    TARGET_LINUX_APPIMAGE, "0.3.9",
                ).is_err(),
                "malformed {platform} digest was accepted"
            );
            let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
            doc["artifacts"][platform]["filename"] = json!("Aquilum-malicious-package");
            assert!(
                inspect_authenticated(
                    &serde_json::to_vec(&doc).unwrap(), bytes,
                    TARGET_LINUX_APPIMAGE, "0.3.9",
                ).is_err(),
                "wrong {platform} filename was accepted"
            );
        }
    }

    #[test]
    fn signed_filename_and_size_cannot_be_substituted() {
        let bytes = b"new";
        let raw = fixture(bytes, "0.3.10");
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        doc["artifacts"]["linux-x86_64-appimage"]["filename"] = json!("Aquilum_0.3.10_x86_64.AppImage");
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        doc["artifacts"]["linux-x86_64-appimage"]["size"] = json!(99999999);
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
    }

    // Run through the dedicated signed-candidate step in Application CI.
    // It generates two disposable Minisign keypairs and signs the exact JSON
    // fixture with the standalone CLI; this test exercises the real Rust
    // minisign_verify wrapper, not a stub or a mocked signature validator.
    #[test]
    #[ignore = "CI supplies a real ephemeral Minisign-signed manifest"]
    fn ephemeral_minisign_fixture_accepts_exact_bytes() {
        use base64::Engine;
        use std::{env, fs, path::PathBuf};
        let root = PathBuf::from(
            env::var("AKULA_SIGNED_CANDIDATE_TEST_ROOT")
                .expect("real Minisign fixture directory required by CI"),
        );
        let manifest = fs::read(root.join("manifest.json")).unwrap();
        let signature = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("manifest.minisig")).unwrap());
        let key = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("publisher.pub")).unwrap());
        let wrong_key = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("other.pub")).unwrap());
        let payload = fs::read(root.join("payload.AppImage")).unwrap();

        let candidate = admit_with_trusted_key(
            &manifest, &signature, payload.clone(), &key,
            TARGET_LINUX_APPIMAGE, "0.3.9",
        ).expect("the actual CLI-generated Minisign signature must verify");
        assert_eq!(candidate.bytes(), payload.as_slice());
        assert_eq!(candidate.version(), "0.3.10");
        assert_eq!(candidate.source_commit(), "a".repeat(40));
        assert_eq!(candidate.sha256(), digest_hex(&payload));

        let mut changed = payload.clone();
        changed[0] ^= 0x01;
        let error = admit_with_trusted_key(
            &manifest, &signature, changed, &key, TARGET_LINUX_APPIMAGE, "0.3.9",
        ).unwrap_err();
        assert!(error.contains("downloaded bytes"), "{error}");

        let error = admit_with_trusted_key(
            &manifest, &signature, payload.clone(), &wrong_key,
            TARGET_LINUX_APPIMAGE, "0.3.9",
        ).unwrap_err();
        assert!(error.contains("authentication failed"), "{error}");

        let mut altered_manifest = manifest.clone();
        altered_manifest.push(b' ');
        let error = admit_with_trusted_key(
            &altered_manifest, &signature, payload.clone(), &key,
            TARGET_LINUX_APPIMAGE, "0.3.9",
        ).unwrap_err();
        assert!(error.contains("authentication failed"), "{error}");

        let error = admit_with_trusted_key(
            &manifest, &signature, payload.clone(), &key,
            TARGET_LINUX_APPIMAGE, "0.3.10",
        ).unwrap_err();
        assert!(error.contains("not strictly newer"), "{error}");

        #[cfg(target_os = "linux")]
        {
            // A real ephemeral Minisign signature is the *only* positive
            // source for this sealed candidate; stage its exact bytes on
            // Linux under a genuine flock and rehash the written inode.
            let installation = root.join("signed-stage-fixture");
            fs::create_dir(&installation).unwrap();
            let active = installation.join("akula-app_0.3.9_x86_64.AppImage");
            let unrelated = installation.join("Aquilum.AppImage");
            let old = b"old installed Akula, never overwritten during staging";
            let sentinel = b"pre-existing Aquilum bytes: do not mutate";
            fs::write(&active, old).unwrap();
            fs::write(&unrelated, sentinel).unwrap();

            // Even a structurally and cryptographically valid TEST candidate
            // cannot activate production staging with the shipped empty key.
            let denied = admit_with_trusted_key(
                &manifest, &signature, payload.clone(), &key,
                TARGET_LINUX_APPIMAGE, "0.3.9",
            ).unwrap();
            let denied_path = installation.join("never-installed-akula.AppImage");
            let refused = crate::updater::linux_signed_stage::stage_signed_candidate(
                &denied_path, "fedcba9876543210fedcba9876543210", denied,
            ).err().expect("production staging must refuse even a valid test key");
            assert!(refused.contains("HOLD"));
            assert!(!denied_path.exists());

            let lease = crate::updater::linux_signed_stage::stage_verified_for_test(
                &active, "0123456789abcdef0123456789abcdef", candidate,
            ).expect("exact signed bytes must be admitted to a locked stage");
            assert_eq!(lease.version(), "0.3.10");
            assert_eq!(lease.source_commit(), "a".repeat(40));
            assert_eq!(lease.digest(), digest_hex(&payload));
            lease.rehash().expect("disk inode must still match signed candidate");
            let staged = lease.stage_path().join("candidate.AppImage");
            assert_eq!(fs::read(&staged).unwrap(), payload);
            assert_eq!(fs::read(&active).unwrap(), old.to_vec());
            assert_eq!(fs::read(&unrelated).unwrap(), sentinel.to_vec());

            let mut damaged = payload.clone();
            damaged[0] ^= 1;
            fs::write(&staged, &damaged).unwrap();
            assert!(lease.rehash().unwrap_err().contains("staged disk bytes"));
            assert_eq!(fs::read(&active).unwrap(), old.to_vec());
            assert_eq!(fs::read(&unrelated).unwrap(), sentinel.to_vec());

            fs::remove_file(&staged).unwrap();
            std::os::unix::fs::symlink(&unrelated, &staged).unwrap();
            assert!(lease.rehash().is_err(), "symlink must fail closed");
            assert_eq!(fs::read(&unrelated).unwrap(), sentinel.to_vec());

            // Repair the disposable stage and exercise actual rename-publish
            // followed by missing-GUI-ACK rollback under the SAME flock.
            // No production feature or publisher trust setting is modified.
            fs::remove_file(&staged).unwrap();
            fs::write(&staged, &payload).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
            fs::File::open(&staged).unwrap().sync_all().unwrap();
            fs::File::open(lease.stage_path()).unwrap().sync_all().unwrap();
            lease.rehash().unwrap();
            lease.prove_missing_ack_rollback()
                .expect("signed test candidate must preserve the previous version after missing ACK");
            assert_eq!(fs::read(&active).unwrap(), old.to_vec());
            assert_eq!(fs::read(lease.stage_path().join("failed.AppImage")).unwrap(), payload);
            assert_eq!(fs::read(&unrelated).unwrap(), sentinel.to_vec());
            drop(lease);
            println!("AKULA_SIGNED_LINUX_STAGE_PASS real_minisign=1 fsync=1 disk_sha256=1 tamper_hold=1 aquilum_intact=1");
            println!("AKULA_SIGNED_LINUX_ROLLBACK_PASS signed_candidate=1 v1_restored=1 v2_preserved_as_failed=1");
        }
    }

    #[test]
    #[ignore = "CI supplies the real ephemeral Minisign-signed Windows manifest"]
    fn ephemeral_minisign_windows_fixture_accepts_signed_exact_bytes() {
        use base64::Engine;
        use std::{env, fs, path::PathBuf};
        let root = PathBuf::from(
            env::var("AKULA_SIGNED_CANDIDATE_TEST_ROOT")
                .expect("Windows Minisign fixture root required by CI"),
        );
        let manifest = fs::read(root.join("manifest-windows.json")).unwrap();
        let signature = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("manifest-windows.minisig")).unwrap());
        let key = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("publisher.pub")).unwrap());
        let wrong_key = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("other.pub")).unwrap());
        let payload = fs::read(root.join("payload.NSIS")).unwrap();

        let offer = signed_offer_with_trusted_key(
            &manifest, &signature, &key,
            TARGET_WINDOWS_NSIS, "0.3.9",
        ).expect("the exact signed Windows metadata must authenticate")
            .expect("newer Windows release expected");
        assert_eq!(offer.version, "0.3.10");
        assert_eq!(offer.size, payload.len() as u64);
        assert_eq!(offer.sha256, digest_hex(&payload));

        let admitted = admit_with_trusted_key(
            &manifest, &signature, payload.clone(), &key,
            TARGET_WINDOWS_NSIS, "0.3.9",
        ).expect("real Minisign-signed Windows installer bytes must authenticate");
        assert_eq!(admitted.bytes(), payload.as_slice());
        assert_eq!(admitted.version(), "0.3.10");
        assert_eq!(admitted.sha256(), digest_hex(&payload));

        assert!(signed_offer_with_trusted_key(
            &manifest, &signature, &wrong_key,
            TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap_err().contains("authentication failed"));
        assert!(signed_offer_with_trusted_key(
            &manifest, &signature, &key,
            TARGET_LINUX_APPIMAGE, "0.3.9",
        ).is_err(), "Windows-only signed bytes must not authorize a Linux offer");

        let mut changed_bytes = payload.clone();
        changed_bytes[0] ^= 1;
        assert!(admit_with_trusted_key(
            &manifest, &signature, changed_bytes, &key,
            TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap_err().contains("downloaded bytes"));
        assert!(admit_with_trusted_key(
            &manifest, &signature, payload.clone(), &wrong_key,
            TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap_err().contains("authentication failed"));
        let mut changed_manifest = manifest.clone();
        changed_manifest.push(b' ');
        assert!(admit_with_trusted_key(
            &changed_manifest, &signature, payload.clone(), &key,
            TARGET_WINDOWS_NSIS, "0.3.9",
        ).unwrap_err().contains("authentication failed"));
        assert!(admit_with_trusted_key(
            &manifest, &signature, payload, &key,
            TARGET_WINDOWS_NSIS, "0.3.10",
        ).unwrap_err().contains("not strictly newer"));
        println!("AKULA_WINDOWS_REAL_MINISIGN_ADMISSION_PASS correct_key=1 exact_nsis=1 wrong_key=0 downgrade=0");
    }

    #[test]
    fn production_admission_refuses_before_parse_without_publisher_root() {
        let result = admit(b"{}", "", b"any bytes".to_vec());
        assert!(result.unwrap_err().contains("channel remains disabled"));
        let failed = admit_with_trusted_key(
            b"{}", "", b"any bytes".to_vec(),
            "", TARGET_LINUX_APPIMAGE, "0.3.9");
        assert!(failed.unwrap_err().contains("publisher key is not installed"));
    }
}
