use base64::Engine;
use std::io::Read;

// RELEASE-SIGNED-DISCOVERY-01: authenticate metadata before inspecting ANY
// remote version, artifact name, or inferred URL. This does not enable
// installation; the Windows transaction/Bridge/ACK remain separate gates.
#[allow(dead_code)]
mod signed_candidate;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) use signed_candidate::SignedCandidate;

pub(super) const ENDPOINT: &str =
    "https://github.com/Lalalalendia/Akula/releases/latest/download/latest.json";

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
const PINNED_RELEASE_BASE: &str =
    "https://github.com/Lalalalendia/Akula/releases/download";

// This reads UNTRUSTED raw bytes without deserializing them. The manifest is
// interpreted only after the exact raw bytes have passed publisher signature
// verification in signed_candidate::offer.
fn bounded_read(mut reader: impl Read, maximum: u64) -> Result<Vec<u8>, String> {
    let mut limited = (&mut reader).take(maximum + 1);
    let mut bytes = Vec::new();
    limited.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > maximum {
        return Err("updater signed metadata stream is empty or exceeds approved size".into());
    }
    Ok(bytes)
}

fn read_manifest(reader: impl Read) -> Result<Vec<u8>, String> {
    bounded_read(reader, MAX_MANIFEST_BYTES)
}

// cargo-packager stores a complete Minisign .minisig as base64 text in .sig.
// A malformed or oversized remote signature is not a metadata witness.
fn read_signature(reader: impl Read) -> Result<String, String> {
    let raw = bounded_read(reader, MAX_SIGNATURE_BYTES)?;
    let text = std::str::from_utf8(&raw)
        .map_err(|_| "updater signature contains non-UTF8 bytes")?
        .trim();
    if text.is_empty()
        || !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=')
    {
        return Err("updater signature is not a single base64 Minisign document".into());
    }
    Ok(text.to_owned())
}

// Only an offer that already passed the pinned-key signature and exact
// schema/filename checks may reach URL construction. Never use a URL from
// untrusted JSON or an inherited Aquilum endpoint.
fn canonical_asset_url(offer: &signed_candidate::VerifiedOffer) -> String {
    format!(
        "{PINNED_RELEASE_BASE}/v{}/{}",
        offer.version, offer.filename,
    )
}

// Akula-owned Minisign public key. The secret key is never in source.
// Key ID (human-readable Minisign comment): D7A66B6D4046F6C5.
pub(super) const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEQ3QTY2QjZENDA0NkY2QzUKUldURjlrWkFiV3VtMXh2SFFsdEljaUw0T0pUakVmd1krZG4wVmJ1REVmaEx1eWcrMFIzTjQzMi8K";

/// Minimal signed-NSIS beta: Windows installed builds use the already
/// pinned Akula publisher key. No Linux/macOS release is authorized here.
/// Every Windows payload still requires independently valid Minisign signatures
/// on the exact manifest and installer, plus signed size/SHA-256 admission.
/// This grants NO automatic NSIS crash rollback (manual repair for beta).
pub(super) const fn channel_ready() -> bool {
    cfg!(all(target_os = "windows", feature = "installed"))
}

#[derive(Clone, Debug)]
pub(super) struct Release {
    pub(super) version: String,
    pub(super) url: String,
    pub(super) signature: String,
    pub(super) manifest: Vec<u8>,
    pub(super) manifest_signature: String,
    pub(super) sha256: String,
    pub(super) size: u64,
}

// Keep the old numerical comparison ONLY for existing non-security tests.
// Production discovery uses strict three-component versions in SignedOffer.
#[cfg(test)]
pub(super) fn newer(candidate: &str, current: &str) -> bool {
    fn version(text: &str) -> Vec<u64> {
        text.trim().trim_start_matches('v')
            .split(['.', '-', '+']).map_while(|part| part.parse().ok())
            .collect()
    }
    version(candidate) > version(current)
}

pub(super) fn find() -> Result<Option<Release>, String> {
    // Do not even make a network call while this product channel is HOLD.
    if !channel_ready() {
        return Err("HOLD: Akula signed-release channel not approved".into());
    }

    let agent = aquilum_core::web_agent::web_agent("updates");
    let manifest = read_manifest(
        agent.get(ENDPOINT).call().map_err(|e| e.to_string())?.into_reader(),
    )?;
    // The signature URL is a fixed suffix of the first-party manifest URL.
    let signature_url = format!("{ENDPOINT}.sig");
    let manifest_signature = read_signature(
        agent.get(&signature_url).call().map_err(|e| e.to_string())?.into_reader(),
    )?;

    // This verifies the exact metadata bytes with the installed publisher
    // key BEFORE considering version or selecting any download location.
    let Some(offer) = signed_candidate::offer(&manifest, &manifest_signature)? else {
        return Ok(None);
    };
    let url = canonical_asset_url(&offer);
    let package_signature_url = format!("{url}.sig");
    let signature = read_signature(
        agent.get(&package_signature_url).call().map_err(|e| e.to_string())?.into_reader(),
    )?;
    Ok(Some(Release {
        version: offer.version,
        url,
        signature,
        manifest,
        manifest_signature,
        sha256: offer.sha256,
        size: offer.size,
    }))
}

/// The native installer receives only candidates authenticated under the
/// exact signed manifest. This does NOT authorize a Windows apply/rollback.
pub(super) fn admit_signed_payload(
    manifest: &[u8],
    signature: &str,
    payload: Vec<u8>,
) -> Result<signed_candidate::SignedCandidate, String> {
    signed_candidate::admit(manifest, signature, payload)
}

fn decoded(text: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim()).map_err(|e| e.to_string())?;
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

pub(super) fn verify(data: &[u8], signature: &str, key: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::decode(&decoded(key)?)
        .map_err(|e| e.to_string())?;
    let signature = minisign_verify::Signature::decode(&decoded(signature)?)
        .map_err(|e| e.to_string())?;
    key.verify(data, &signature, true).map_err(|e| e.to_string())
}

#[cfg(test)]
mod input_tests {
    use super::*;

    #[test]
    fn owner_pinned_akula_minisign_public_key_decodes_but_cannot_enable_updates() {
        let formatted = decoded(PUBLIC_KEY).expect("owner-provided base64 is malformed");
        assert_eq!(
            formatted.lines().next(),
            Some("untrusted comment: minisign public key: D7A66B6D4046F6C5"),
        );
        assert_eq!(formatted.lines().count(), 2, "expected exactly one Minisign key record");
        minisign_verify::PublicKey::decode(&formatted)
            .expect("owner-provided key must be a valid Minisign public key");
        assert_eq!(
            channel_ready(),
            cfg!(all(target_os = "windows", feature = "installed")),
            "only signed installed Windows beta may use Akula-owned releases",
        );
    }

    #[test]
    fn unsigned_manifest_is_never_interpreted_by_the_reader() {
        // The reader is deliberately a bounded raw-byte reader. Even invalid
        // JSON MUST remain uninterpreted until successful Minisign verification.
        let raw = b"{invalid untrusted json";
        assert_eq!(read_manifest(raw.as_slice()).unwrap(), raw);
        assert!(signed_candidate::offer(raw, "bad-signature").is_err());
    }

    #[test]
    fn oversized_and_empty_manifest_are_refused_before_interpretation() {
        let payload = vec![b' '; MAX_MANIFEST_BYTES as usize + 1];
        assert!(read_manifest(payload.as_slice()).unwrap_err().contains("exceeds"));
        assert!(read_manifest(b"".as_slice()).is_err());
    }

    #[test]
    fn exact_manifest_limit_does_not_allow_an_extra_byte() {
        let bytes = vec![b' '; MAX_MANIFEST_BYTES as usize];
        assert_eq!(read_manifest(bytes.as_slice()).unwrap().len() as u64, MAX_MANIFEST_BYTES);
        let mut larger = bytes;
        larger.push(b' ');
        assert!(read_manifest(larger.as_slice()).is_err());
    }

    #[test]
    fn invalid_signatures_fail_closed_at_the_transport_boundary() {
        let huge = vec![b'a'; MAX_SIGNATURE_BYTES as usize + 1];
        assert!(read_signature(huge.as_slice()).is_err());
        for value in [b"".as_slice(), b" \n".as_slice(), b"bad signature".as_slice()] {
            assert!(read_signature(value).is_err());
        }
        assert!(read_signature([0xff].as_slice()).is_err());
        assert_eq!(read_signature(b"YWJjZA==\n".as_slice()).unwrap(), "YWJjZA==");
    }

    #[test]
    fn only_validated_offer_can_derive_first_party_asset_url() {
        let offer = signed_candidate::VerifiedOffer {
            version: "0.3.10".into(),
            filename: "Akula_0.3.10_x64-setup.exe".into(),
            sha256: "a".repeat(64),
            size: 12,
            source_commit: "b".repeat(40),
        };
        assert_eq!(
            canonical_asset_url(&offer),
            "https://github.com/Lalalalendia/Akula/releases/download/v0.3.10/Akula_0.3.10_x64-setup.exe",
        );
    }

    #[test]
    fn disabled_update_channel_performs_no_network_lookup() {
        assert!(find().unwrap_err().contains("HOLD"));
    }
}
