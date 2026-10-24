//! Immutable, preverified bootstrap-helper data and fixed root staging source.
//! No SSH call, sudo, process spawn, service activation, or owner approval.
//! The signer key MUST come from a separately pinned client release authority.
use crate::server_release_lab::{
    verify_signed_release, ReleaseError, ReleaseFloor, SignedRelease, TrustedReleaseSigner,
};
use std::io::Cursor;
use std::sync::Arc;

const FRAME_DOMAIN: &[u8] = b"AKULA_ROOT_STAGE_V1\0";
const BOOTSTRAP_SIGNER: &str = "akula-root-bootstrap-v1";
/// Deliberately UNPROVISIONED. The production public key must be reviewed,
/// pinned in signed Akula release source and matched against an independently
/// signed publisher artifact. Never load this from an SSH response, manifest,
/// environment variable, or caller argument.
const PRODUCTION_BOOTSTRAP_PUBKEY: Option<[u8; 32]> = None;
const MAX_BOOTSTRAP_BYTES: usize = 64 * 1024 * 1024;
const PIN_PLACEHOLDER: &str = "PINNED_BOOTSTRAP_ED25519_HEX = \"\"";
const SOURCE: &str = include_str!("../../../scripts/server-bootstrap/root_stage_v1.py");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootStageError {
    UntrustedBootstrapSigner,
    BadArtifact,
    BadSource,
    Release(ReleaseError),
}

/// Inert bytes only. Consuming this value cannot authorize an SSH session.
#[must_use]
pub struct VerifiedRootStageBundle {
    frame: Arc<[u8]>,
    fixed_root_source: String,
    helper_package_sha256: [u8; 32],
}

impl VerifiedRootStageBundle {
    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    /// Share verified bytes with the bounded SSH writer without a second
    /// potentially 64 MiB allocation.
    #[cfg(all(target_os = "windows", feature = "server-root-stage-ssh-lab"))]
    pub(crate) fn shared_frame(&self) -> Arc<[u8]> {
        Arc::clone(&self.frame)
    }

    /// This is compiled source, with only the already independently pinned
    /// Ed25519 public key substituted as lowercase ASCII hexadecimal.
    pub fn fixed_root_source(&self) -> &str {
        &self.fixed_root_source
    }

    /// Signed privileged-helper digest used by the separately reviewed B0
    /// v3 plan and compared before a root SSH process may be launched.
    pub fn package_sha256(&self) -> [u8; 32] {
        self.helper_package_sha256
    }
}

/// Production cannot be prepared until a publisher trust key is pinned
/// independently of the candidate signer passed to this function.
pub fn prepare_root_stage_bundle(
    signed: &SignedRelease,
    pinned: &TrustedReleaseSigner,
    previous_floor: ReleaseFloor,
    package: &[u8],
    now_unix: u64,
) -> Result<VerifiedRootStageBundle, RootStageError> {
    let expected_key = PRODUCTION_BOOTSTRAP_PUBKEY
        .ok_or(RootStageError::UntrustedBootstrapSigner)?;
    prepare_with_fixed_anchor(
        signed, pinned, expected_key, previous_floor, package, now_unix,
    )
}

/// The factory cannot be called from a production code path with a caller-
/// chosen key. Tests exercise this private path against an independent known
/// fixture key rather than weakening the production release gate.
fn prepare_with_fixed_anchor(
    signed: &SignedRelease,
    pinned: &TrustedReleaseSigner,
    expected_key: [u8; 32],
    previous_floor: ReleaseFloor,
    package: &[u8],
    now_unix: u64,
) -> Result<VerifiedRootStageBundle, RootStageError> {
    if pinned.signer_id != BOOTSTRAP_SIGNER
        || signed.manifest.signer_id != BOOTSTRAP_SIGNER
        || expected_key == [0; 32]
        || pinned.ed25519_public != expected_key
    {
        return Err(RootStageError::UntrustedBootstrapSigner);
    }
    if package.len() < 64
        || package.len() > MAX_BOOTSTRAP_BYTES
        || package.get(..7) != Some(b"\x7fELF\x02\x01\x01".as_slice())
        || package.get(18..20) != Some(b"\x3e\x00".as_slice())
    {
        return Err(RootStageError::BadArtifact);
    }
    let verified = verify_signed_release(signed, pinned, previous_floor, now_unix)
        .map_err(RootStageError::Release)?;
    verified
        .verify_package_stream(&mut Cursor::new(package))
        .map_err(RootStageError::Release)?;
    let manifest = signed.manifest.canonical_signed_bytes();
    if manifest.is_empty() || manifest.len() > 256 {
        return Err(RootStageError::BadArtifact);
    }
    if SOURCE.matches(PIN_PLACEHOLDER).count() != 1 {
        return Err(RootStageError::BadSource);
    }
    let mut hex = String::with_capacity(64);
    for byte in expected_key {
        use std::fmt::Write as _;
        write!(&mut hex, "{byte:02x}").map_err(|_| RootStageError::BadSource)?;
    }
    let source = SOURCE.replacen(
        PIN_PLACEHOLDER,
        &format!("PINNED_BOOTSTRAP_ED25519_HEX = \"{hex}\""),
        1,
    );
    let mut frame = Vec::with_capacity(FRAME_DOMAIN.len() + 2 + manifest.len() + 64 + package.len());
    frame.extend_from_slice(FRAME_DOMAIN);
    frame.extend_from_slice(&(manifest.len() as u16).to_be_bytes());
    frame.extend_from_slice(&manifest);
    frame.extend_from_slice(&signed.signature);
    frame.extend_from_slice(package);
    Ok(VerifiedRootStageBundle {
        frame: frame.into(),
        fixed_root_source: source,
        helper_package_sha256: signed.manifest.package_sha256,
    })
}

#[cfg(test)]
mod tests;
