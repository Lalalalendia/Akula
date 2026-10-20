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
    #[serde(rename = "windows-x86_64")]
    windows: Artifact,
    #[serde(rename = "linux-x86_64-appimage")]
    appimage: Artifact,
    #[serde(rename = "linux-x86_64-deb")]
    deb: Artifact,
    #[serde(rename = "linux-x86_64-rpm")]
    rpm: Artifact,
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
pub(super) struct SignedCandidate {
    bytes: Vec<u8>,
    version: String,
    source_commit: String,
    sha256: String,
}

impl SignedCandidate {
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn version(&self) -> &str {
        &self.version
    }

    pub(super) fn source_commit(&self) -> &str {
        &self.source_commit
    }

    pub(super) fn sha256(&self) -> &str {
        &self.sha256
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

/// Strictly validate already-authenticated metadata and package bytes.
/// This function is NOT itself a signature verifier; it must never be used
/// directly as a release authorization path.
fn inspect_authenticated(
    raw: &[u8],
    bytes: &[u8],
    expected_platform: &str,
    running_version: &str,
) -> Result<(String, String, String), String> {
    if raw.is_empty() || raw.len() > MAX_MANIFEST_BYTES {
        return Err(hold("signed manifest size limit"));
    }
    if expected_platform != TARGET_LINUX_APPIMAGE {
        return Err(hold("unsupported or unexpected platform for automatic apply"));
    }
    let manifest: SignedManifest =
        serde_json::from_slice(raw).map_err(|_| hold("ambiguous, malformed or unknown-field manifest"))?;
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
    if next <= current {
        return Err(hold("release is not strictly newer than the installed version"));
    }
    // Exactly these four platform fields must exist, with no aliases.
    // Their own payload shapes are parsed by deny_unknown_fields.
    // Automatic replacement is limited to the Linux AppImage path.
    let _ = (&manifest.artifacts.windows, &manifest.artifacts.deb, &manifest.artifacts.rpm);
    let asset = &manifest.artifacts.appimage;
    let required_name = format!("akula-app_{}_x86_64.AppImage", manifest.version);
    if asset.filename != required_name {
        return Err(hold("signed platform filename does not identify Akula AppImage"));
    }
    if !valid_hex(&asset.sha256, 64)
        || asset.size == 0
        || asset.size > MAX_LINUX_IMAGE_BYTES as u64
        || asset.size != bytes.len() as u64
    {
        return Err(hold("signed asset hash format, size or budget mismatch"));
    }
    if digest_hex(bytes) != asset.sha256 {
        return Err(hold("downloaded bytes are not the signed platform asset"));
    }
    Ok((manifest.version, manifest.source_commit, asset.sha256.clone()))
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
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (manifest, signature, payload);
        Err(hold("only installed Linux x86_64 AppImage admission is supported"))
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
    fn unknown_fields_and_missing_platforms_refuse_admission() {
        let bytes = b"new";
        let raw = fixture(bytes, "0.3.10");
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        doc["artifacts"].as_object_mut().unwrap().remove("linux-x86_64-rpm");
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
        let mut doc: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        doc["artifacts"]["other-os"] = json!({"filename":"other","sha256":"c".repeat(64),"size":12});
        assert!(inspect_authenticated(&serde_json::to_vec(&doc).unwrap(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
        let repeated = String::from_utf8(raw).unwrap().replace("\"product\":\"Akula\"",
            "\"product\":\"Akula\",\"product\":\"Aquilum\"");
        assert!(inspect_authenticated(repeated.as_bytes(), bytes, TARGET_LINUX_APPIMAGE, "0.3.9").is_err());
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
