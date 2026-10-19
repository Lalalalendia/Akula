use base64::Engine;
use std::io::Read;

pub(super) const ENDPOINT: &str = "https://github.com/Lalalalendia/Akula/releases/latest/download/latest.json";

const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// Signed-update metadata is untrusted until the pinned Akula key verifies
/// the downloaded asset. Never allow the HTTP JSON parser unbounded input.
fn parse_manifest(reader: impl Read) -> Result<serde_json::Value, String> {
    let mut limited = reader.take(MAX_MANIFEST_BYTES + 1);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err("updater release manifest exceeded approved size".into());
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
// No Akula signing key is provisioned. Never reuse the Aquilum signing key.
pub(super) const PUBLIC_KEY: &str = "";

/// Intentionally fail closed until a separately signed Akula release stream
/// exists, with a tested upgrade/migration path and a pinned Akula public key.
pub(super) const fn channel_ready() -> bool {
    false
}

#[derive(Clone)]
pub(super) struct Release {
    pub(super) version: String,
    pub(super) url: String,
    pub(super) signature: String,
}

fn target() -> Option<String> {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return None;
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        return None;
    };
    Some(format!("{os}-{arch}"))
}

fn version(text: &str) -> Vec<u64> {
    text.trim().trim_start_matches('v').split(['.', '-', '+']).map_while(|part| part.parse().ok()).collect()
}

pub(super) fn newer(candidate: &str, current: &str) -> bool {
    version(candidate) > version(current)
}

pub(super) fn find() -> Result<Option<Release>, String> {
    let agent = aquilum_core::web_agent::web_agent("updates");
    let manifest = parse_manifest(agent.get(ENDPOINT).call().map_err(|e| e.to_string())?.into_reader())?;
    let version = manifest["version"].as_str().ok_or("в latest.json нет версии")?.to_owned();
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let target = target().ok_or("платформа не поддерживается")?;
    let platform = &manifest["platforms"][&target];
    let url = platform["url"].as_str().ok_or_else(|| format!("в latest.json нет сборки для {target}"))?.to_owned();
    let signature = platform["signature"].as_str().ok_or("в latest.json нет подписи")?.to_owned();
    Ok(Some(Release { version, url, signature }))
}

fn decoded(text: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

pub(super) fn verify(data: &[u8], signature: &str, key: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::decode(&decoded(key)?).map_err(|e| e.to_string())?;
    let signature = minisign_verify::Signature::decode(&decoded(signature)?).map_err(|e| e.to_string())?;
    key.verify(data, &signature, true).map_err(|e| e.to_string())
}


#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn small_well_formed_manifest_is_accepted() {
        let parsed = parse_manifest(br#"{"version":"0.3.0","platforms":{}}"#.as_slice()).unwrap();
        assert_eq!(parsed["version"], "0.3.0");
    }

    #[test]
    fn oversized_manifest_is_refused_before_json_decode() {
        let payload = vec![b' '; MAX_MANIFEST_BYTES as usize + 1];
        let error = parse_manifest(payload.as_slice()).unwrap_err();
        assert!(error.contains("exceeded"));
    }

    #[test]
    fn malformed_manifest_is_refused() {
        assert!(parse_manifest(b"{not json".as_slice()).is_err());
    }

    #[test]
    fn exact_manifest_budget_is_not_an_unbounded_read() {
        let mut bytes = br#"{"version":"1"}"#.to_vec();
        bytes.resize(MAX_MANIFEST_BYTES as usize, b' ');
        assert!(parse_manifest(bytes.as_slice()).is_ok());
        bytes.push(b' ');
        assert!(parse_manifest(bytes.as_slice()).is_err());
    }
}
