// VIDEO-18b-18a: five-component Windows video candidate admission.
//!
// The ordinary signed Akula NSIS offer authenticates one installer, not the
// installed five-file generation. This separate contract authenticates an
// exact *set* of five PE byte identities and a strictly increasing version.
// It is intentionally disconnected from update discovery, apply, and commit.
// The five-EXE channel remains HOLD. No result here authorizes privilege,
// installation, a transaction, a rendered-frame ACK, or cleanup of old bytes.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

const SCHEMA: u8 = 1;
const PRODUCT: &str = "Akula";
const PLATFORM: &str = "windows-x86_64";
const PURPOSE: &str = "protected-video-five-exe";
const MAX_MANIFEST: usize = 8192;
const MIN_PE_BYTES: u64 = 512;
const MAX_PE_BYTES: u64 = 512 * 1024 * 1024;
const NAMES: [&str; 5] = [
    "akula-app.exe",
    "shiva-pdf-preview.exe",
    "akula-video-mux.exe",
    "akula-yt-dlp.exe",
    "akula-quickjs.exe",
];

// Separate from the ordinary TWO-EXE Windows channel. This must not be
// replaced by super::channel_ready(), which intentionally remains true in
// normal installed Windows builds without the experimental video feature.
const VIDEO_FIVE_CHANNEL_READY: bool = false;

fn hold(why: &str) -> String {
    format!("VIDEO18B18_SIGNED_FIVE_HOLD: {why}")
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    product: String,
    platform: String,
    purpose: String,
    version: String,
    source_commit: String,
    cargo_lock_sha256: String,
    components: Vec<Component>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Component {
    filename: String,
    sha256: String,
    size: u64,
}

/// Fields are private and only returned after *real* Minisign verification.
/// The digest binds a future generation-specific ACK to the exact signed
/// manifest bytes, not just a freely caller-supplied version string.
#[derive(Debug)]
pub struct SignedFive {
    manifest_sha256: String,
    manifest: Manifest,
}
impl SignedFive {
    pub fn version(&self) -> &str {
        &self.manifest.version
    }
    pub fn generation_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    /// Authenticated build provenance, never supplied by a raw caller.
    pub fn source_commit(&self) -> &str {
        &self.manifest.source_commit
    }
    pub fn cargo_lock_sha256(&self) -> &str {
        &self.manifest.cargo_lock_sha256
    }

    /// Verify the exact first-presented-GUI receipt against this authenticated
    /// five-PE manifest, never against an arbitrary caller-supplied EXE digest.
    ///
    /// This is read-only: the trusted external Windows supervisor must separately
    /// pin the OS child HANDLE/creation identity, private inherited pipe, first
    /// visible frame, deadline, protected install root, lock and journal.
    /// A valid receipt must NOT authorize confirm_candidate or publication.
    pub fn verify_first_presented_frame_ack(
        &self,
        receipt: &[u8],
        expected_nonce: &str,
        pinned_os_child_pid: u32,
    ) -> Result<(), String> {
        if receipt.is_empty() || receipt.len() > 256 {
            return Err(hold("empty or oversized first-frame receipt"));
        }
        if !lowercase_hex(expected_nonce, 32) || pinned_os_child_pid <= 1 {
            return Err(hold("invalid supervisor nonce or pinned OS child PID"));
        }
        // Both version and SHA originate only from the cryptographically
        // admitted SignedFive object. Exact byte comparison rejects CRLF,
        // duplicate/trailing receipts, wrong field order or alternate encodings.
        let expected = format!(
            "AKULA-WIN-FIRST-PRESENTED-V1\\t{expected_nonce}\\t{}\\t{}\\t{pinned_os_child_pid}\\n",
            self.version(),
            self.generation_sha256(),
        );
        if receipt != expected.as_bytes() {
            return Err(hold("first-frame receipt not bound to signed five-PE generation"));
        }
        Ok(())
    }

    /// Verify a caller-selected staging directory under an external trusted
    /// install lock. This neither grants authority to that path nor prevents a
    /// subsequent TOCTOU mutation. The privileged controller must enforce
    /// the protected root, ACL, lock, and rehash at its own activation point.
    pub fn verify_stage(&self, current: &Path) -> Result<(), String> {
        check_path(current, true)?;
        let entries = fs::read_dir(current)
            .map_err(|e| hold(&format!("cannot enumerate staged generation: {e}")))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| hold(&format!("cannot read staged entry: {e}")))?;
        if entries.len() != NAMES.len() {
            return Err(hold("generation has missing or extra entries"));
        }
        for item in &entries {
            let name = item.file_name();
            let Some(name) = name.to_str() else {
                return Err(hold("non-UTF8 or ambiguous staged member"));
            };
            if !NAMES.contains(&name) {
                return Err(hold("unknown member in staged generation"));
            }
        }
        for record in &self.manifest.components {
            let file = current.join(&record.filename);
            let (size, sha) = hash_pe(&file)?;
            if size != record.size || sha != record.sha256 {
                return Err(hold("staged PE size or SHA-256 differs from publisher-signed generation"));
            }
        }
        Ok(())
    }
}

fn lowercase_hex(raw: &str, len: usize) -> bool {
    raw.len() == len
        && raw.bytes().all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
}

fn semver3(raw: &str) -> Result<[u32; 3], String> {
    if raw.len() > 32 {
        return Err(hold("version too long"));
    }
    let parts: Vec<_> = raw.split('.').collect();
    if parts.len() != 3 {
        return Err(hold("version must have three numeric parts"));
    }
    let mut parsed = [0u32; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty()
            || part.len() > 10
            || (part.len() > 1 && part.starts_with('0'))
            || !part.bytes().all(|x| x.is_ascii_digit())
        {
            return Err(hold("ambiguous or invalid numeric version"));
        }
        parsed[i] = part.parse::<u32>().map_err(|_| hold("numeric version overflow"))?;
    }
    Ok(parsed)
}

fn inspect_authenticated(raw: &[u8], installed_floor: &str) -> Result<Manifest, String> {
    if raw.is_empty() || raw.len() > MAX_MANIFEST {
        return Err(hold("empty or oversized five-PE manifest"));
    }
    let manifest: Manifest =
        serde_json::from_slice(raw).map_err(|_| hold("unknown field or malformed manifest"))?;
    if manifest.schema != SCHEMA
        || manifest.product != PRODUCT
        || manifest.platform != PLATFORM
        || manifest.purpose != PURPOSE
    {
        return Err(hold("wrong schema/product/platform or bundle purpose"));
    }
    if !lowercase_hex(&manifest.source_commit, 40)
        || !lowercase_hex(&manifest.cargo_lock_sha256, 64)
    {
        return Err(hold("missing or invalid signed source/lock provenance"));
    }
    if semver3(&manifest.version)? <= semver3(installed_floor)? {
        return Err(hold("signed version is not strictly newer than installed floor"));
    }
    if manifest.components.len() != NAMES.len() {
        return Err(hold("signed manifest requires exactly five PE records"));
    }
    for (record, expected) in manifest.components.iter().zip(NAMES) {
        if record.filename != expected
            || !lowercase_hex(&record.sha256, 64)
            || !(MIN_PE_BYTES..=MAX_PE_BYTES).contains(&record.size)
        {
            return Err(hold("unexpected PE name/order, digest or byte-size"));
        }
    }
    Ok(manifest)
}

/// The only signature-positive constructor, testable with an ephemeral key.
/// Production callers cannot replace the pinned key or version-floor source.
fn verify(raw: &[u8], signature: &str, trusted_key: &str) -> Result<(), String> {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let key_bytes = b64.decode(trusted_key).map_err(|_| hold("invalid Minisign public key encoding"))?;
    let sig_bytes = b64.decode(signature).map_err(|_| hold("invalid Minisign signature encoding"))?;
    let key_text = std::str::from_utf8(&key_bytes).map_err(|_| hold("invalid Minisign public key text"))?;
    let sig_text = std::str::from_utf8(&sig_bytes).map_err(|_| hold("invalid Minisign signature text"))?;
    let key = minisign_verify::PublicKey::decode(key_text)
        .map_err(|_| hold("invalid publisher key"))?;
    let sig = minisign_verify::Signature::decode(sig_text)
        .map_err(|_| hold("invalid detached signature"))?;
    key.verify(raw, &sig, true).map_err(|_| hold("publisher signature authentication failed"))
}

fn admit_with_key(
    raw: &[u8], signature: &str, trusted_key: &str, installed_floor: &str,
) -> Result<SignedFive, String> {
    if trusted_key.is_empty() {
        return Err(hold("missing trusted publisher key"));
    }
    if raw.is_empty() || raw.len() > MAX_MANIFEST {
        return Err(hold("manifest byte budget"));
    }
    verify(raw, signature, trusted_key)
        .map_err(|_| hold("publisher signature authentication failed"))?;
    let manifest = inspect_authenticated(raw, installed_floor)?;
    let manifest_sha256 = Sha256::digest(raw).iter().map(|b| format!("{b:02x}")).collect();
    Ok(SignedFive { manifest_sha256, manifest })
}

/// PR-only injection of an ephemeral verification key for protected Windows
/// integration tests. Not a product entrypoint: the standalone Cargo package
/// is publish=false and no Akula runtime Cargo graph imports it.
#[cfg(all(windows, feature = "lab-only-signature"))]
pub fn admit_hosted_lab_signature(
    raw: &[u8], signature: &str, test_key: &str, installed_floor: &str,
) -> Result<SignedFive, String> {
    for (name, expected) in [
        ("GITHUB_ACTIONS", "true"),
        ("RUNNER_OS", "Windows"),
        ("RUNNER_ENVIRONMENT", "github-hosted"),
        ("GITHUB_EVENT_NAME", "pull_request"),
    ] {
        if std::env::var(name).ok().as_deref() != Some(expected) {
            return Err(hold("disposable hosted Windows PR proof only"));
        }
    }
    admit_with_key(raw, signature, test_key, installed_floor)
}

/// No video release authority is currently enabled, even though Akula has an
/// existing pinned publisher key for its ordinary two-component NSIS beta.
#[allow(dead_code)]
pub(crate) fn admit(
    raw: &[u8], signature: &str, installed_floor: &str,
) -> Result<SignedFive, String> {
    if !VIDEO_FIVE_CHANNEL_READY {
        return Err(hold("five-EXE release channel is disabled"));
    }
    let _ = (raw, signature, installed_floor);
    Err(hold("five-EXE publisher binding is not connected"))
}

fn check_path(path: &Path, dir: bool) -> Result<(), String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| hold(&format!("staged path missing: {e}")))?;
    let ty = meta.file_type();
    if ty.is_symlink() || (if dir { !ty.is_dir() } else { !ty.is_file() }) {
        return Err(hold("nonregular staged path or filesystem alias"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(hold("Windows reparse point in candidate generation"));
        }
    }
    Ok(())
}

fn hash_pe(path: &Path) -> Result<(u64, String), String> {
    check_path(path, false)?;
    let mut file = File::open(path)
        .map_err(|e| hold(&format!("cannot open staged PE: {e}")))?;
    let initial = file.metadata().map_err(|e| hold(&format!("cannot stat PE: {e}")))?;
    if !(MIN_PE_BYTES..=MAX_PE_BYTES).contains(&initial.len()) {
        return Err(hold("staged PE exceeds byte budget"));
    }
    let mut mz = [0u8; 2];
    file.read_exact(&mut mz).map_err(|_| hold("missing PE header"))?;
    if &mz != b"MZ" {
        return Err(hold("staged component is not an MZ executable"));
    }
    let mut hasher = Sha256::new();
    hasher.update(mz);
    let mut bytes = 2u64;
    let mut block = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut block).map_err(|e| hold(&format!("cannot stream staged PE: {e}")))?;
        if n == 0 { break; }
        bytes = bytes.checked_add(n as u64).ok_or_else(|| hold("PE length overflow"))?;
        if bytes > MAX_PE_BYTES { return Err(hold("PE grew beyond budget")); }
        hasher.update(&block[..n]);
    }
    let after = file.metadata().map_err(|e| hold(&format!("cannot restat PE: {e}")))?;
    check_path(path, false)?;
    if bytes != initial.len() || bytes != after.len() {
        return Err(hold("PE changed size during verification"));
    }
    Ok((bytes, hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::{Value, json};
    use std::path::PathBuf;

    fn fixture() -> (PathBuf, Vec<u8>, String, String) {
        let root = PathBuf::from(std::env::var("AKULA_VIDEO18B18_SIGNED_FIXTURE_ROOT")
            .expect("VIDEO18B18_HOLD: ephemeral Minisign fixture root required by CI"));
        let manifest = fs::read(root.join("five-manifest.json")).unwrap();
        let signature = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("five-manifest.minisig")).unwrap());
        let key = base64::engine::general_purpose::STANDARD
            .encode(fs::read(root.join("test-publisher.pub")).unwrap());
        (root, manifest, signature, key)
    }

    #[test]
    fn exact_ephemeral_minisign_authority_binds_all_five_pe_bytes() {
        let (root, bytes, signature, key) = fixture();
        let admitted = admit_with_key(&bytes, &signature, &key, "0.3.20").unwrap();
        assert_eq!(admitted.version(), "0.3.21");
        assert_eq!(admitted.generation_sha256(), Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>());
        let stage = root.join("current");
        admitted.verify_stage(&stage).unwrap();
        let mut changed = bytes.clone();
        changed.push(b' ');
        assert!(admit_with_key(&changed, &signature, &key, "0.3.20")
            .unwrap_err().contains("authentication failed"));
        let mut wrong_key = key.clone();
        wrong_key.push('x');
        assert!(admit_with_key(&bytes, &signature, &wrong_key, "0.3.20").is_err());
        assert!(admit_with_key(&bytes, &signature, &key, "0.3.21")
            .unwrap_err().contains("not strictly newer"));
        assert!(admit_with_key(&bytes, &signature, &key, "0.3.22")
            .unwrap_err().contains("not strictly newer"));
        assert!(admit(&bytes, &signature, "0.3.20")
            .unwrap_err().contains("channel is disabled"));

        let first = stage.join(NAMES[0]);
        let original = fs::read(&first).unwrap();
        let mut corrupted = original.clone();
        corrupted[3] ^= 1;
        fs::write(&first, &corrupted).unwrap();
        assert!(admitted.verify_stage(&stage).is_err(), "same-size PE corruption accepted");
        fs::write(&first, &original).unwrap();
        admitted.verify_stage(&stage).unwrap();

        let extraneous = stage.join("extra.exe");
        fs::write(&extraneous, b"MZ").unwrap();
        assert!(admitted.verify_stage(&stage).is_err(), "extra component accepted");
        fs::remove_file(&extraneous).unwrap();

        let missing = stage.join(NAMES[4]);
        let old = fs::read(&missing).unwrap();
        fs::remove_file(&missing).unwrap();
        assert!(admitted.verify_stage(&stage).is_err(), "missing fifth component accepted");
        fs::write(&missing, old).unwrap();
        admitted.verify_stage(&stage).unwrap();

        #[cfg(unix)]
        {
            let alias = root.join("alias");
            std::os::unix::fs::symlink(&stage, &alias).unwrap();
            assert!(admitted.verify_stage(&alias).is_err(), "symlinked tree accepted");
            fs::remove_file(alias).unwrap();
        }
    }

    #[test]
    fn first_frame_ack_is_bound_to_authenticated_five_pe_manifest_not_gui_exe_hash() {
        let (root, bytes, signature, key) = fixture();
        let admitted = admit_with_key(&bytes, &signature, &key, "0.3.20").unwrap();
        let nonce = "0123456789abcdef0123456789abcdef";
        let pid = 4567u32;
        let exact = format!(
            "AKULA-WIN-FIRST-PRESENTED-V1\\t{nonce}\\t{}\\t{}\\t{pid}\\n",
            admitted.version(), admitted.generation_sha256(),
        );
        admitted.verify_first_presented_frame_ack(exact.as_bytes(), nonce, pid).unwrap();

        let executable_hash: String = Sha256::digest(
            fs::read(root.join("current/akula-app.exe")).unwrap(),
        ).iter().map(|b| format!("{b:02x}")).collect();
        assert_ne!(executable_hash, admitted.generation_sha256(),
            "GUI-only digest must not masquerade as signed generation");
        let gui_only = exact.replace(admitted.generation_sha256(), &executable_hash);
        assert!(admitted.verify_first_presented_frame_ack(gui_only.as_bytes(), nonce, pid).is_err());

        for mutated in [
            exact.replace(nonce, "00000000000000000000000000000000"),
            exact.replace("\\t0.3.21\\t", "\\t0.3.22\\t"),
            exact.replace(&pid.to_string(), "4568"),
            exact.replace(admitted.generation_sha256(), &"f".repeat(64)),
            exact.replace("\\t", " "),
            exact.replace("\\n", "\\r\\n"),
            format!("{exact}{exact}"),
            format!("{exact}trailing"),
        ] {
            assert!(admitted.verify_first_presented_frame_ack(
                mutated.as_bytes(), nonce, pid,
            ).is_err(), "untrusted/malformed ACK was accepted: {mutated:?}");
        }
        let mut invalid_binary = exact.as_bytes().to_vec();
        invalid_binary.push(0);
        assert!(admitted.verify_first_presented_frame_ack(
            &invalid_binary, nonce, pid,
        ).is_err());
        assert!(admitted.verify_first_presented_frame_ack(
            &[b'x'; 257], nonce, pid,
        ).is_err());
        for invalid_nonce in ["", "0", "A123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcde\\n",
            "0123456789abcdef0123456789abcde_"] {
            assert!(admitted.verify_first_presented_frame_ack(
                exact.as_bytes(), invalid_nonce, pid,
            ).is_err());
        }
        for invalid_pid in [0, 1, pid + 1] {
            assert!(admitted.verify_first_presented_frame_ack(
                exact.as_bytes(), nonce, invalid_pid,
            ).is_err());
        }
    }

    #[test]
    fn authenticated_shape_rejects_aliases_rollback_and_ambiguous_extra_fields() {
        let (_root, bytes, _signature, _key) = fixture();
        let mut data: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(inspect_authenticated(&bytes, "0.3.20").is_ok());
        for (key, value) in [
            ("platform", json!("linux-x86_64")),
            ("product", json!("Aquilum")),
            ("purpose", json!("ordinary-windows-nsis")),
            ("schema", json!(2)),
            ("version", json!("0.3.20")),
        ] {
            let before = data[key].clone();
            data[key] = value;
            assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err(), "{key}");
            data[key] = before;
        }
        for bad in ["0.3.021", "v0.3.21", "0.3.21-beta", "4294967296.0.0"] {
            data["version"] = json!(bad);
            assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err(), "{bad}");
        }
        data["version"] = json!("0.3.21");
        data["components"].as_array_mut().unwrap().swap(0, 1);
        assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err());
        data["components"].as_array_mut().unwrap().swap(0, 1);
        data["components"][4]["filename"] = json!("AKULA-QUICKJS.exe");
        assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err());
        data["components"][4]["filename"] = json!("akula-quickjs.exe");
        let duplicate = data["components"][4].clone();
        data["components"].as_array_mut().unwrap().push(duplicate);
        assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err());
        data["components"].as_array_mut().unwrap().pop();
        data["components"][0]["sha256"] = json!("A".repeat(64));
        assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err());
        data["components"][0]["sha256"] = json!("a".repeat(64));
        data["unexpected"] = json!(true);
        assert!(inspect_authenticated(&serde_json::to_vec(&data).unwrap(), "0.3.20").is_err());
    }
}
