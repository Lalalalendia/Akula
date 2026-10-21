//! B1b4b0: a real root-owned local CAS witness journal with an independently
//! protected test signing key. Never installation authority.
//!
//! The trust directory and ledger are OS-owned, separate from the staged
//! package and client IPC. This protects against nonroot writers, NOT against
//! a compromised UID0, disk rollback, forged clock or malicious provisioner.
//! All paths/signing material are fixed, root-provisioned disposable CI state.
//! The private key is never supplied over a socket or returned to a caller.
//! The B1b1c ledger still supplies cooperating-process CAS + fsync durability;
//! this module adds OS-owner checks and signs only an actually recorded NEW
//! CAS revision. It refuses retry/AlreadyRecorded as a second attestation.
use crate::server_installer_cas_fence_lab::{
    signed_owner_sha256, SignedWitnessClaim, TrustedWitnessSigner, WitnessClaim,
};
use crate::server_installer_pinned_release_lab::{PinError, RootVerifiedFd};
use crate::server_owner_approval_lab::{
    ExternalApprovalScope, SignedOwnerApproval, TrustedOwnerApprover,
};
use crate::server_release_floor_lab::ProposedFloor;
use crate::server_staging_lab::StageReceipt;
use crate::server_witness_lab::{
    InertWitnessLedgerLab, WitnessCheckpoint, WitnessDecision, WitnessError,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::{File, Metadata};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use zeroize::Zeroize;

const ID: &str = "akula-b1b4b-ci-witness";
const ENROLLMENT: &[u8] = b"AKULA-B1B4B-CI-ENROLLED\n";
// B1d0 production witness enrollment is deliberately NOT the disposable
// B1b4b CI ceremony. The root-installing service must never accept a CI
// witness seed, marker, signing identity or caller-chosen ledger directory.
#[cfg(feature = "server-bootstrap-b1d0-durable-node")]
const PROD_ID: &str = "akula-b1d0-provisioned-owner-witness";
#[cfg(feature = "server-bootstrap-b1d0-durable-node")]
const PROD_ENROLLMENT: &[u8] = b"AKULA-B1D0-KEY-ENROLLED\n";
#[cfg(feature = "server-bootstrap-b1d0-durable-node")]
const PROD_WITNESS_ROOT: &str = "/var/lib/akula/server-bootstrap";
const MAX_RECEIPT_SECONDS: u64 = 30;

#[derive(Debug)]
pub enum OsWitnessError {
    Io(std::io::Error),
    TrustBoundary,
    SignatureKey,
    Pinned(PinError),
    PackageMismatch,
    Witness(WitnessError),
    StaleCheckpoint,
    DuplicateIsNotGrant,
    NeedsRepair,
    InvalidChallenge,
}
impl From<std::io::Error> for OsWitnessError {
    fn from(v: std::io::Error) -> Self { Self::Io(v) }
}
impl From<WitnessError> for OsWitnessError {
    fn from(v: WitnessError) -> Self { Self::Witness(v) }
}
fn os(v: rustix::io::Errno) -> OsWitnessError {
    OsWitnessError::Io(v.into())
}
fn unchanged(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino() && a.len() == b.len()
        && a.ctime() == b.ctime() && a.ctime_nsec() == b.ctime_nsec()
        && a.mtime() == b.mtime() && a.mtime_nsec() == b.mtime_nsec()
}
fn root_dir(path: &Path) -> Result<File, OsWitnessError> {
    let dir = File::from(open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    let m = dir.metadata()?;
    if !m.is_dir() || m.uid() != 0 || m.nlink() == 0
        || m.permissions().mode() & 0o7777 != 0o700
    { return Err(OsWitnessError::TrustBoundary); }
    Ok(dir)
}
fn pinned_bytes<const N: usize>(
    dir: &File, name: &str, mode: u32,
) -> Result<[u8; N], OsWitnessError> {
    let mut file = File::from(openat(
        dir, name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ).map_err(os)?);
    let before = file.metadata()?;
    if !before.is_file() || before.uid() != 0 || before.nlink() != 1
        || before.permissions().mode() & 0o7777 != mode || before.len() != N as u64
    { return Err(OsWitnessError::TrustBoundary); }
    let mut data = [0u8; N];
    file.read_exact(&mut data)?;
    if !unchanged(&before, &file.metadata()?) {
        data.zeroize();
        return Err(OsWitnessError::TrustBoundary);
    }
    Ok(data)
}
/// Root-owned witness actor, provisioned OUTSIDE candidate/SSH/client traffic.
/// Must be instantiated only in a dedicated OS-owned witness service.
pub struct OsWitnessLab {
    ledger: InertWitnessLedgerLab,
    signer: Ed25519KeyPair,
    trust: TrustedWitnessSigner,
}
impl OsWitnessLab {
    pub fn open(trust_path: &Path, journal_path: &Path) -> Result<Self, OsWitnessError> {
        Self::open_enrolled(trust_path, journal_path, ENROLLMENT, ID)
    }
    // Dedicated, separately enrolled production witness. It is not exposed
    // to a remote SSH/IPC client or UI. All path components must be trusted
    // root-owned directories BEFORE any signing seed can be loaded.
    #[cfg(feature = "server-bootstrap-b1d0-durable-node")]
    pub(crate) fn open_provisioned_first_install() -> Result<Self, OsWitnessError> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err(OsWitnessError::TrustBoundary);
        }
        let mut current = File::from(open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(os)?);
        let filesystem_root = current.metadata()?;
        if !filesystem_root.is_dir() || filesystem_root.uid() != 0 ||
            filesystem_root.nlink() == 0 ||
            filesystem_root.permissions().mode() & 0o7777 != 0o755 {
            return Err(OsWitnessError::TrustBoundary);
        }
        for (component, mode) in [
            ("var", 0o755), ("lib", 0o755),
            ("akula", 0o700), ("server-bootstrap", 0o700),
        ] {
            let next = File::from(openat(
                &current, component,
                OFlags::RDONLY | OFlags::DIRECTORY |
                    OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            ).map_err(os)?);
            let metadata = next.metadata()?;
            if !metadata.is_dir() || metadata.uid() != 0 ||
                metadata.nlink() == 0 ||
                metadata.permissions().mode() & 0o7777 != mode {
                return Err(OsWitnessError::TrustBoundary);
            }
            current = next;
        }
        let root = Path::new(PROD_WITNESS_ROOT);
        Self::open_enrolled(
            &root.join("trust"), &root.join("journal"),
            PROD_ENROLLMENT, PROD_ID,
        )
    }
    fn open_enrolled(
        trust_path: &Path, journal_path: &Path,
        expected_marker: &[u8], witness_id: &str,
    ) -> Result<Self, OsWitnessError> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err(OsWitnessError::TrustBoundary);
        }
        let trust_dir = root_dir(trust_path)?;
        let journal_dir = root_dir(journal_path)?;
        let trust_m = trust_dir.metadata()?;
        let journal_m = journal_dir.metadata()?;
        if trust_m.dev() == journal_m.dev() && trust_m.ino() == journal_m.ino() {
            return Err(OsWitnessError::TrustBoundary);
        }
        // Both enrollment profiles have the same fixed marker length.
        // A different production marker cannot be substituted with a CI key.
        if expected_marker.len() != ENROLLMENT.len() {
            return Err(OsWitnessError::TrustBoundary);
        }
        let marker = pinned_bytes::<{ ENROLLMENT.len() }>(
            &trust_dir, "enrollment", 0o600,
        )?;
        if marker.as_slice() != expected_marker {
            return Err(OsWitnessError::TrustBoundary);
        }
        let public = pinned_bytes::<32>(&trust_dir, "witness.pub", 0o600)?;
        let mut seed = pinned_bytes::<32>(&trust_dir, "witness.seed", 0o600)?;
        let signer = Ed25519KeyPair::from_seed_unchecked(&seed)
            .map_err(|_| OsWitnessError::SignatureKey);
        seed.zeroize();
        let signer = signer?;
        if signer.public_key().as_ref() != public.as_slice() {
            return Err(OsWitnessError::SignatureKey);
        }
        let ledger = InertWitnessLedgerLab::open(journal_path)?;
        let trust = TrustedWitnessSigner {
            witness_id: witness_id.into(),
            ed25519_public: public,
        };
        Ok(Self { ledger, signer, trust })
    }
    pub fn trusted_witness(&self) -> TrustedWitnessSigner {
        self.trust.clone()
    }
    pub fn current_checkpoint(&self) -> Result<WitnessCheckpoint, OsWitnessError> {
        self.ledger.current_checkpoint().map_err(OsWitnessError::Witness)
    }
    /// Exactly one new B1b1c CAS is written, fsynced and read back before the
    /// independently held witness key signs the specific accepted revision.
    ///
    /// Not an install capability. A lost response leaves a consumed approval,
    /// and the caller must HOLD rather than receiving another signed grant.
    #[allow(clippy::too_many_arguments)]
    pub fn attest_new_record(
        &self,
        pinned: &RootVerifiedFd,
        owner: &SignedOwnerApproval,
        trusted_owner: &TrustedOwnerApprover,
        scope: ExternalApprovalScope,
        staged: &StageReceipt,
        proposed: ProposedFloor,
        expected_before: WitnessCheckpoint,
        helper_challenge: [u8; 32],
        now_unix: u64,
    ) -> Result<SignedWitnessClaim, OsWitnessError> {
        if helper_challenge == [0; 32] {
            return Err(OsWitnessError::InvalidChallenge);
        }
        pinned.valid_at(now_unix).map_err(OsWitnessError::Pinned)?;
        if staged.verified_package != pinned.receipt {
            return Err(OsWitnessError::PackageMismatch);
        }
        if self.current_checkpoint()? != expected_before {
            return Err(OsWitnessError::StaleCheckpoint);
        }
        let decision = self.ledger.record_signed_approval(
            owner, trusted_owner, scope, staged, proposed,
            expected_before, now_unix,
        )?;
        let accepted = match decision {
            WitnessDecision::NewlyRecorded { checkpoint } => checkpoint,
            WitnessDecision::AlreadyRecorded { .. } => {
                return Err(OsWitnessError::DuplicateIsNotGrant);
            }
        };
        if self.current_checkpoint()? != accepted {
            return Err(OsWitnessError::NeedsRepair);
        }
        pinned.valid_at(now_unix).map_err(OsWitnessError::Pinned)?;
        let expires_at_unix = now_unix.checked_add(MAX_RECEIPT_SECONDS)
            .ok_or(OsWitnessError::TrustBoundary)?;
        let claim = WitnessClaim {
            witness_id: self.trust.witness_id.clone(),
            before: expected_before,
            after: accepted,
            install_id: scope.install_id.0,
            plan_digest: scope.approved_plan_digest.0,
            host_sha256: scope.pinned_host_key_sha256,
            manifest_fingerprint: pinned.receipt.signed_manifest_fingerprint,
            package_sha256: pinned.receipt.package_sha256,
            signed_owner_sha256: signed_owner_sha256(owner),
            approval_nonce: owner.claim.approval_nonce,
            challenge: helper_challenge,
            issued_at_unix: now_unix,
            expires_at_unix,
        };
        let signature = self.signer.sign(&claim.canonical_bytes())
            .as_ref().try_into().map_err(|_| OsWitnessError::SignatureKey)?;
        Ok(SignedWitnessClaim { claim, signature })
    }
}
