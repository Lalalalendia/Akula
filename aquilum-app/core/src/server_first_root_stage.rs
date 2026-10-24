//! Immutable, preverified bootstrap-helper data and fixed root staging source.
//! No SSH call, sudo, process spawn, service activation, or owner approval.
//! The signer key MUST come from a separately pinned client release authority.
use crate::server_release_lab::{
    verify_signed_release, ReleaseError, ReleaseFloor, SignedRelease, TrustedReleaseSigner,
};
use std::io::Cursor;

const FRAME_DOMAIN: &[u8] = b"AKULA_ROOT_STAGE_V1\0";
const BOOTSTRAP_SIGNER: &str = "akula-root-bootstrap-v1";
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
    frame: Vec<u8>,
    fixed_root_source: String,
}

impl VerifiedRootStageBundle {
    pub fn frame(&self) -> &[u8] {
        &self.frame
    }

    /// This is compiled source, with only the already independently pinned
    /// Ed25519 public key substituted as lowercase ASCII hexadecimal.
    pub fn fixed_root_source(&self) -> &str {
        &self.fixed_root_source
    }
}

pub fn prepare_root_stage_bundle(
    signed: &SignedRelease,
    pinned: &TrustedReleaseSigner,
    previous_floor: ReleaseFloor,
    package: &[u8],
    now_unix: u64,
) -> Result<VerifiedRootStageBundle, RootStageError> {
    if pinned.signer_id != BOOTSTRAP_SIGNER
        || signed.manifest.signer_id != BOOTSTRAP_SIGNER
        || pinned.ed25519_public == [0; 32]
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
    for byte in pinned.ed25519_public {
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
        frame,
        fixed_root_source: source,
    })
}

#[cfg(test)]
mod tests;
