//! AKULA-SERVER-BOOTSTRAP B1a: inert signed-release admission.
//!
//! This module verifies a B1 candidate's *metadata signature*, its immutable
//! byte digest, freshness and a caller-provided trusted release floor. It does
//! NOT download, install, execute, open paths, persist a floor, authorize a
//! remote host, update a service or change any user vault.
//!
//! Security boundaries:
//! 1. The Ed25519 trust anchor MUST be installed by a trusted operator /
//!    separate release-key ceremony, never fetched from the SSH host/package.
//! 2. The caller MUST supply a rollback floor from separately durable storage;
//!    this pure module cannot prevent rollback of that storage.
//! 3. The verified stream receipt is NOT a pinned file descriptor. An installer
//!    must separately prevent a hostile writer replacing verified bytes.
//! 4. This is a laboratory contract, NOT an Akula Node signing key or release.

use ring::{constant_time, digest, signature};
use serde::Serialize;
use std::io::Read;

pub const MAX_NODE_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_MANIFEST_VALIDITY_SECONDS: u64 = 90 * 24 * 60 * 60;

const SIGNING_DOMAIN: &[u8] = b"Akula Node B1a signed release manifest v1\0";
const FINGERPRINT_DOMAIN: &str = "Akula Node B1a authenticated release identity v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum TargetPlatform {
    LinuxX86_64,
    // No ARM64, Windows, RouterOS, Android, OCI or Docker support is claimed.
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReleaseManifestV1 {
    pub platform: TargetPlatform,
    pub version: Version,
    /// Strictly increasing release authority epoch. Never derive from a
    /// package-provided "latest" label or SSH server observation.
    pub release_sequence: u64,
    /// Exact original binary package size.
    pub package_size: u64,
    /// SHA-256 of the exact original artifact bytes, not a compressed
    /// preview or untrusted HTTP checksum sidecar.
    pub package_sha256: [u8; 32],
    pub not_before_unix: u64,
    pub expires_unix: u64,
    pub signer_id: String,
}

impl ReleaseManifestV1 {
    /// Fixed-field canonical signing domain. No JSON serialization ambiguity,
    /// reordered map keys, whitespace, floats, locale, path or shell strings.
    pub fn canonical_signed_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(SIGNING_DOMAIN.len() + 128);
        out.extend_from_slice(SIGNING_DOMAIN);
        out.push(1u8); // schema version
        out.push(match self.platform { TargetPlatform::LinuxX86_64 => 1 });
        for field in [self.version.major, self.version.minor, self.version.patch] {
            out.extend_from_slice(&field.to_be_bytes());
        }
        for field in [
            self.release_sequence, self.package_size,
            self.not_before_unix, self.expires_unix,
        ] {
            out.extend_from_slice(&field.to_be_bytes());
        }
        out.extend_from_slice(&self.package_sha256);
        out.push(u8::try_from(self.signer_id.len()).unwrap_or(u8::MAX));
        out.extend_from_slice(self.signer_id.as_bytes());
        out
    }

    fn validate(&self, now_unix: u64) -> Result<(), ReleaseError> {
        // Restrict the signer namespace to a short stable identifier.
        let id = self.signer_id.as_bytes();
        if id.is_empty() || id.len() > 64
            || !id.iter().all(|b| b.is_ascii_alphanumeric()
                || *b == b'.' || *b == b'_' || *b == b'-')
            || self.release_sequence == 0
            || self.package_size == 0
            || self.package_size > MAX_NODE_PACKAGE_BYTES
            || self.package_sha256.iter().all(|b| *b == 0)
        {
            return Err(ReleaseError::InvalidMetadata);
        }
        let duration = self.expires_unix.checked_sub(self.not_before_unix)
            .ok_or(ReleaseError::InvalidMetadata)?;
        if duration == 0 || duration > MAX_MANIFEST_VALIDITY_SECONDS {
            return Err(ReleaseError::InvalidMetadata);
        }
        if now_unix < self.not_before_unix { return Err(ReleaseError::NotYetValid); }
        if now_unix >= self.expires_unix { return Err(ReleaseError::Expired); }
        Ok(())
    }
}

/// Explicitly trusted release-signing public key, NOT a TLS/SSH host key and
/// NOT a NodeId, VaultId, DeviceId or MCP principal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustedReleaseSigner {
    pub signer_id: String,
    pub ed25519_public: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedRelease {
    pub manifest: ReleaseManifestV1,
    pub signature: [u8; 64],
}

/// Separately persisted, operator/host-owned minimum acceptable release.
/// A caller must never accept this value from candidate package metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleaseFloor {
    pub sequence: u64,
    pub version: Version,
    /// If reusing the already accepted sequence, the exact signed manifest
    /// identity MUST agree. Prevent same-epoch signer's equivocation.
    pub signed_manifest_fingerprint: [u8; 32],
}

impl ReleaseFloor {
    pub const fn initial() -> Self {
        Self {
            sequence: 0,
            version: Version { major: 0, minor: 0, patch: 0 },
            signed_manifest_fingerprint: [0u8; 32],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseError {
    InvalidMetadata,
    UntrustedSigner,
    BadSignature,
    NotYetValid,
    Expired,
    RollbackRefused,
    StreamIo,
    SizeMismatch,
    PackageDigestMismatch,
}

/// Proof that the pinned signing key authenticated a typed manifest. This
/// alone is NOT approval to install; it has no filesystem write capability.
#[derive(Clone, Debug)]
pub struct VerifiedRelease {
    manifest: ReleaseManifestV1,
    fingerprint: [u8; 32],
}

impl VerifiedRelease {
    pub fn manifest(&self) -> &ReleaseManifestV1 { &self.manifest }

    pub fn release_floor_candidate(&self) -> ReleaseFloor {
        ReleaseFloor {
            sequence: self.manifest.release_sequence,
            version: self.manifest.version,
            signed_manifest_fingerprint: self.fingerprint,
        }
    }

    /// Hash the *complete* artifact stream, including trailing bytes. Refuse
    /// content larger or smaller than the signed exact package_size. There
    /// is no path lookup, temp file, artifact execution or network access.
    pub fn verify_package_stream<R: Read>(
        &self, reader: &mut R
    ) -> Result<VerifiedPackageReceipt, ReleaseError> {
        let mut sha = digest::Context::new(&digest::SHA256);
        let mut seen = 0u64;
        let mut buf = [0u8; 32 * 1024];
        loop {
            let n = reader.read(&mut buf).map_err(|_| ReleaseError::StreamIo)?;
            if n == 0 { break; }
            seen = seen.checked_add(n as u64)
                .ok_or(ReleaseError::SizeMismatch)?;
            if seen > self.manifest.package_size {
                return Err(ReleaseError::SizeMismatch);
            }
            sha.update(&buf[..n]);
        }
        if seen != self.manifest.package_size { return Err(ReleaseError::SizeMismatch); }
        let observed = sha.finish();
        if constant_time::verify_slices_are_equal(
            observed.as_ref(), &self.manifest.package_sha256
        ).is_err() {
            return Err(ReleaseError::PackageDigestMismatch);
        }
        Ok(VerifiedPackageReceipt {
            signed_manifest_fingerprint: self.fingerprint,
            byte_length: seen,
            package_sha256: self.manifest.package_sha256,
            proposed_floor: self.release_floor_candidate(),
        })
    }
}

/// This receipt describes a verified *stream*, not a protected on-disk inode.
/// B1b must pin a sealed artifact and journal approval before installation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifiedPackageReceipt {
    pub signed_manifest_fingerprint: [u8; 32],
    pub byte_length: u64,
    pub package_sha256: [u8; 32],
    pub proposed_floor: ReleaseFloor,
}

pub fn verify_signed_release(
    signed: &SignedRelease,
    trusted: &TrustedReleaseSigner,
    previous: ReleaseFloor,
    now_unix: u64,
) -> Result<VerifiedRelease, ReleaseError> {
    signed.manifest.validate(now_unix)?;
    if signed.manifest.signer_id != trusted.signer_id
        || trusted.ed25519_public.iter().all(|x| *x == 0)
    {
        return Err(ReleaseError::UntrustedSigner);
    }
    let bytes = signed.manifest.canonical_signed_bytes();
    signature::UnparsedPublicKey::new(&signature::ED25519, trusted.ed25519_public)
        .verify(&bytes, &signed.signature)
        .map_err(|_| ReleaseError::BadSignature)?;

    let mut hash = blake3::Hasher::new_derive_key(FINGERPRINT_DOMAIN);
    hash.update(&bytes);
    hash.update(&signed.signature);
    let fingerprint: [u8; 32] = *hash.finalize().as_bytes();

    if signed.manifest.release_sequence < previous.sequence
        || signed.manifest.version < previous.version
    {
        return Err(ReleaseError::RollbackRefused);
    }
    if signed.manifest.release_sequence == previous.sequence {
        // Never reinterpret the same release epoch as a different digest,
        // even if signed by the same key.
        if previous.sequence == 0 || constant_time::verify_slices_are_equal(
            &fingerprint, &previous.signed_manifest_fingerprint
        ).is_err() {
            return Err(ReleaseError::RollbackRefused);
        }
    }
    Ok(VerifiedRelease { manifest: signed.manifest.clone(), fingerprint })
}
