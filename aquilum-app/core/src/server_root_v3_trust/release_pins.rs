//! B1-E2E-07: first-release admission from INDEPENDENT root-owned state.
//!
//! All identities are opened under the same fixed OS-owned trust directory.
//! SSH, a candidate's signature file and the desktop UI cannot supply the
//! expected issuer, manifest fingerprint, package hash or release sequence.
//! Read-only: no CAS, root helper dispatch, /opt writes or systemd execution.
use super::*;
use crate::server_installer_pinned_release_lab::{
    verify_root_staged_dir, PinError, RootPinnedReleaseTrust,
};
use crate::server_release_lab::{ReleaseFloor, TrustedReleaseSigner};
use rustix::fs::{openat, Mode, OFlags};
use std::time::{SystemTime, UNIX_EPOCH};

const APPROVAL_DOMAIN: &[u8] = b"AKULA-B1-E2E-07-ROOT-FIRST-B1A-RELEASE-V1\0";
const FIRST_APPROVAL_FILE: &str = "b1a.first.approval";
const FIRST_APPROVAL_MARKER: &str = "b1a.first.enrollment";
const FIRST_APPROVAL_MARKER_BYTES: &[u8] = b"AKULA-B1-E2E-07-ROOT-B1A-FIRST-RELEASE-ENROLLED\n";
const FIRST_RELEASE_STAGE: &str = "b1a-first-stage";

#[derive(Debug)]
pub enum FirstReleaseError {
    Trust(RootEnrollmentError),
    InvalidApproval,
    UnsafeStage,
    Verify(PinError),
    NoSystemClock,
}
impl From<RootEnrollmentError> for FirstReleaseError {
    fn from(e: RootEnrollmentError) -> Self {
        Self::Trust(e)
    }
}
impl From<PinError> for FirstReleaseError {
    fn from(e: PinError) -> Self {
        Self::Verify(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FirstReleaseApproval {
    sequence: u64,
    package_size: u64,
    package_sha256: [u8; 32],
    manifest_fingerprint: [u8; 32],
}
impl FirstReleaseApproval {
    fn decode(data: &[u8]) -> Result<Self, FirstReleaseError> {
        const BODY: usize = 1 + 8 + 8 + 32 + 32;
        if data.len() != APPROVAL_DOMAIN.len() + BODY || !data.starts_with(APPROVAL_DOMAIN) {
            return Err(FirstReleaseError::InvalidApproval);
        }
        let mut pos = APPROVAL_DOMAIN.len();
        if take::<1>(data, &mut pos)? != [1] {
            return Err(FirstReleaseError::InvalidApproval);
        }
        let sequence = u64::from_be_bytes(take::<8>(data, &mut pos)?);
        let package_size = u64::from_be_bytes(take::<8>(data, &mut pos)?);
        let package_sha256 = take::<32>(data, &mut pos)?;
        let manifest_fingerprint = take::<32>(data, &mut pos)?;
        if pos != data.len()
            || sequence == 0
            || package_size == 0
            || package_sha256 == [0; 32]
            || manifest_fingerprint == [0; 32]
        {
            return Err(FirstReleaseError::InvalidApproval);
        }
        Ok(Self {
            sequence,
            package_size,
            package_sha256,
            manifest_fingerprint,
        })
    }
    #[cfg(test)]
    fn encode(self) -> Vec<u8> {
        let mut v = APPROVAL_DOMAIN.to_vec();
        v.push(1);
        v.extend_from_slice(&self.sequence.to_be_bytes());
        v.extend_from_slice(&self.package_size.to_be_bytes());
        v.extend_from_slice(&self.package_sha256);
        v.extend_from_slice(&self.manifest_fingerprint);
        v
    }
}

/// Derive ALL expected release identity fields from the same fixed UID0
/// enrollment, not a second caller-injected RootPinnedReleaseTrust.
fn verify_first_from_trust_fd(
    trust: &File,
    now_unix: u64,
) -> Result<RootVerifiedFd, FirstReleaseError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(FirstReleaseError::Trust(RootEnrollmentError::NotRoot));
    }
    let policy = RootEnrolledV3::from_trust_fd(trust)?;
    if read_root_file(trust, FIRST_APPROVAL_MARKER, 128)? != FIRST_APPROVAL_MARKER_BYTES {
        return Err(FirstReleaseError::InvalidApproval);
    }
    let raw = read_root_file(trust, FIRST_APPROVAL_FILE, 256)?;
    let approval = FirstReleaseApproval::decode(&raw)?;
    let stage = File::from(
        openat(
            trust,
            FIRST_RELEASE_STAGE,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|e| RootEnrollmentError::Io(e.into()))?,
    );
    if !root_dir_ok(&stage.metadata().map_err(RootEnrollmentError::Io)?, 0o700) {
        return Err(FirstReleaseError::UnsafeStage);
    }
    let signer = TrustedReleaseSigner {
        signer_id: policy.release_signer_id,
        ed25519_public: policy.release_pub,
    };
    let trust_anchor = RootPinnedReleaseTrust {
        trusted_signer: signer,
        previous_floor: ReleaseFloor::initial(),
        approved_manifest_fingerprint: approval.manifest_fingerprint,
        approved_package_sha256: approval.package_sha256,
        approved_release_sequence: approval.sequence,
    };
    let verified = verify_root_staged_dir(&stage, &trust_anchor, now_unix)?;
    if verified.receipt.byte_length != approval.package_size {
        return Err(FirstReleaseError::InvalidApproval);
    }
    Ok(verified)
}

mod enrollment;
pub use enrollment::admin_enroll_first_b1a_fixed_root_once;

/// Fixed production entry point. No path, verifier key, expected fingerprint
/// or clock value is accepted from caller/SSH. Privileged state must already
/// have been independently enrolled by the operator.
///
/// The result is only an already-verified pinned FD. It is NOT install,
/// activation or owner authorization, and no SSH-accessible release enrollment writer exists.
#[allow(dead_code)]
pub fn open_provisioned_first_b1a_release() -> Result<RootVerifiedFd, FirstReleaseError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(FirstReleaseError::Trust(RootEnrollmentError::NotRoot));
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| FirstReleaseError::NoSystemClock)?
        .as_secs();
    let trust = open_fixed_trust_fd()?;
    verify_first_from_trust_fd(&trust, now)
}

#[cfg(test)]
mod tests;
