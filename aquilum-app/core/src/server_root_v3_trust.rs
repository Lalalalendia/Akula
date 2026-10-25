//! B1-E2E-07: read-only root-owned B0 v3 trust enrollment.
//!
//! This module NEVER creates enrollment, a credential, a root helper process,
//! a file commit, or an installer operation. All policy values are read from
//! a fixed pre-existing UID0-owned path. A candidate's JSON/SSH data cannot
//! substitute for the independently enrolled root trust.
use crate::server_bootstrap_lab::{MachineArch, SupportedOs};
use crate::server_installer_pinned_release_lab::RootVerifiedFd;
use crate::server_owner_approval_lab::TrustedOwnerApprover;
use crate::server_owner_approval_wire::OwnerApprovalWireV1;
use crate::server_root_v3_guard::{
    verify_root_b0_v3_scope, CheckedRootV3Scope, RootIndependentPins, RootPlanError,
};
use rustix::fs::{open, openat, Mode, OFlags};
use std::fs::{File, Metadata};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

const ENROLLMENT: &[u8] = b"AKULA-B1-E2E-07-ROOT-V3-POLICY-ENROLLED\n";
const WITNESS_ENROLLMENT: &[u8] = b"AKULA-B1D0-KEY-ENROLLED\n";
const DOMAIN: &[u8] = b"AKULA-B1-E2E-07-ROOT-TRUST-POLICY-V3\0";
const MAX_POLICY_SIZE: u64 = 512;

#[derive(Debug)]
pub enum RootEnrollmentError {
    NotRoot,
    Io(std::io::Error),
    UntrustedPath,
    InvalidEnrollment,
    InvalidPolicy,
    ReleaseIdentityMismatch,
    InvalidVerifiedRelease,
    UnsupportedHost,
    InvalidOwnerClock,
    OwnerFrame(crate::server_owner_approval_wire::WireError),
    Review(RootPlanError),
}
impl From<std::io::Error> for RootEnrollmentError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
fn os(error: rustix::io::Errno) -> RootEnrollmentError {
    RootEnrollmentError::Io(error.into())
}
fn root_dir_ok(meta: &Metadata, mode: u32) -> bool {
    meta.is_dir()
        && meta.uid() == 0
        && meta.nlink() != 0
        && (meta.permissions().mode() & 0o7777) == mode
}
fn root_file_ok(meta: &Metadata, mode: u32, max: u64) -> bool {
    meta.is_file()
        && meta.uid() == 0
        && meta.nlink() == 1
        && (meta.permissions().mode() & 0o7777) == mode
        && meta.len() <= max
}
fn stable(before: &Metadata, after: &Metadata) -> bool {
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.ctime() == after.ctime()
        && before.ctime_nsec() == after.ctime_nsec()
}
fn read_root_file(dir: &File, leaf: &str, max: u64) -> Result<Vec<u8>, RootEnrollmentError> {
    let file = File::from(
        openat(
            dir,
            leaf,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    let before = file.metadata()?;
    if !root_file_ok(&before, 0o600, max) || before.len() == 0 {
        return Err(RootEnrollmentError::UntrustedPath);
    }
    let mut content = Vec::new();
    (&file).take(max + 1).read_to_end(&mut content)?;
    // The checked inode must remain identical while the data is read.
    let after = file.metadata()?;
    if !stable(&before, &after) || content.len() as u64 != before.len() {
        return Err(RootEnrollmentError::UntrustedPath);
    }
    Ok(content)
}
fn take<const N: usize>(data: &[u8], offset: &mut usize) -> Result<[u8; N], RootEnrollmentError> {
    let end = offset
        .checked_add(N)
        .ok_or(RootEnrollmentError::InvalidPolicy)?;
    let bytes = data
        .get(*offset..end)
        .ok_or(RootEnrollmentError::InvalidPolicy)?;
    *offset = end;
    bytes
        .try_into()
        .map_err(|_| RootEnrollmentError::InvalidPolicy)
}
fn take_identity(data: &[u8], offset: &mut usize) -> Result<String, RootEnrollmentError> {
    let n = take::<1>(data, offset)?[0] as usize;
    if !(1..=64).contains(&n) {
        return Err(RootEnrollmentError::InvalidPolicy);
    }
    let end = offset
        .checked_add(n)
        .ok_or(RootEnrollmentError::InvalidPolicy)?;
    let bytes = data
        .get(*offset..end)
        .ok_or(RootEnrollmentError::InvalidPolicy)?;
    *offset = end;
    if !bytes
        .iter()
        .all(|v| v.is_ascii_alphanumeric() || matches!(*v, b'-' | b'_' | b'.'))
    {
        return Err(RootEnrollmentError::InvalidPolicy);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| RootEnrollmentError::InvalidPolicy)
}

#[derive(Debug)]
pub struct RootEnrolledV3 {
    owner_id: String,
    owner_pub: [u8; 32],
    ssh_host_sha256: [u8; 32],
    helper_sha256: [u8; 32],
    release_signer_id: String,
    release_pub: [u8; 32],
}
impl RootEnrolledV3 {
    /// Return only the independently enrolled owner authority, never an
    /// SSH-supplied TrustedOwnerApprover.
    pub(crate) fn trusted_owner(&self) -> TrustedOwnerApprover {
        TrustedOwnerApprover {
            approver_id: self.owner_id.clone(),
            public_key_ed25519: self.owner_pub,
        }
    }

    fn parse(data: &[u8]) -> Result<Self, RootEnrollmentError> {
        if !data.starts_with(DOMAIN) || data.len() > MAX_POLICY_SIZE as usize {
            return Err(RootEnrollmentError::InvalidPolicy);
        }
        let mut offset = DOMAIN.len();
        if take::<1>(data, &mut offset)? != [3] {
            return Err(RootEnrollmentError::InvalidPolicy);
        }
        let owner_id = take_identity(data, &mut offset)?;
        let owner_pub = take::<32>(data, &mut offset)?;
        let ssh_host_sha256 = take::<32>(data, &mut offset)?;
        let helper_sha256 = take::<32>(data, &mut offset)?;
        let release_signer_id = take_identity(data, &mut offset)?;
        let release_pub = take::<32>(data, &mut offset)?;
        if offset != data.len()
            || [owner_pub, ssh_host_sha256, helper_sha256, release_pub].contains(&[0; 32])
        {
            return Err(RootEnrollmentError::InvalidPolicy);
        }
        Ok(Self {
            owner_id,
            owner_pub,
            ssh_host_sha256,
            helper_sha256,
            release_signer_id,
            release_pub,
        })
    }

    fn from_trust_fd(trust: &File) -> Result<Self, RootEnrollmentError> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err(RootEnrollmentError::NotRoot);
        }
        if !root_dir_ok(&trust.metadata()?, 0o700) {
            return Err(RootEnrollmentError::UntrustedPath);
        }
        if read_root_file(trust, "enrollment", 128)? != WITNESS_ENROLLMENT
            || read_root_file(trust, "b0v3.enrollment", 128)? != ENROLLMENT
        {
            return Err(RootEnrollmentError::InvalidEnrollment);
        }
        Self::parse(&read_root_file(trust, "b0v3.policy", MAX_POLICY_SIZE)?)
    }

    /// Check a REAL previously B1a-verified root-pinned FD and independently
    /// enrolled release key before interpreting the owner review. This is
    /// read-only and does NOT issue or advance any durable CAS transaction.
    pub fn verify_review(
        &self,
        plan_json: &[u8],
        approval: &OwnerApprovalWireV1,
        signature: &[u8; 64],
        verified: &RootVerifiedFd,
        witness_revision: u64,
        local_os: SupportedOs,
        local_arch: MachineArch,
        local_has_systemd: bool,
        now_unix: u64,
    ) -> Result<CheckedRootV3Scope, RootEnrollmentError> {
        if rustix::process::geteuid().as_raw() != 0 {
            return Err(RootEnrollmentError::NotRoot);
        }
        verified
            .valid_at(now_unix)
            .map_err(|_| RootEnrollmentError::InvalidVerifiedRelease)?;
        let (verified_signer_id, verified_signer_key) = verified.verified_publisher();
        if verified_signer_id != self.release_signer_id || verified_signer_key != self.release_pub {
            return Err(RootEnrollmentError::ReleaseIdentityMismatch);
        }
        let version = verified.receipt.proposed_floor.version;
        let version_str = format!("{}.{}.{}", version.major, version.minor, version.patch);
        let pins = RootIndependentPins {
            owner_id: &self.owner_id,
            owner_ed25519: self.owner_pub,
            ssh_host_sha256: self.ssh_host_sha256,
            bootstrap_helper_sha256: self.helper_sha256,
            node_sha256: verified.receipt.package_sha256,
            node_size: verified.receipt.byte_length,
            manifest_fingerprint: verified.receipt.signed_manifest_fingerprint,
            release_signer_id: &self.release_signer_id,
            release_version: &version_str,
            host_os: local_os,
            host_arch: local_arch,
            has_systemd: local_has_systemd,
            witness_revision,
        };
        verify_root_b0_v3_scope(plan_json, approval, signature, pins, now_unix)
            .map_err(RootEnrollmentError::Review)
    }
}

/// Root examines the host independently of the submitted B0 preflight.
/// A non-systemd environment refuses before any production CAS mutation.
pub(crate) fn observed_root_host() -> Result<(SupportedOs, MachineArch, bool), RootEnrollmentError>
{
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot);
    }
    let release = std::fs::read_to_string("/etc/os-release")?;
    let os = parse_local_os_id(&release).ok_or(RootEnrollmentError::UnsupportedHost)?;
    let arch = match std::env::consts::ARCH {
        "x86_64" => MachineArch::X86_64,
        _ => return Err(RootEnrollmentError::UnsupportedHost),
    };
    let has_systemd = Path::new("/run/systemd/system").is_dir();
    if !has_systemd {
        return Err(RootEnrollmentError::UnsupportedHost);
    }
    Ok((os, arch, has_systemd))
}

fn parse_local_os_id(release: &str) -> Option<SupportedOs> {
    let mut result = None;
    for line in release.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            if result.is_some() {
                return None;
            }
            result = match value.trim_matches('"') {
                "ubuntu" => Some(SupportedOs::Ubuntu),
                "debian" => Some(SupportedOs::Debian),
                _ => return None,
            };
        }
    }
    result
}

/// Only the fixed independently provisioned /var/lib/.../trust location.
/// A path passed by SSH, the user, environment or an untrusted manifest is
/// never accepted as a production root authorization source.
pub fn load_provisioned_root_v3_policy() -> Result<RootEnrolledV3, RootEnrollmentError> {
    let trust = open_fixed_trust_fd()?;
    RootEnrolledV3::from_trust_fd(&trust)
}

/// Both the read-only verifier and the explicit administrator-only
/// enrollment command use the same fixed, fully validated path traversal.
fn open_fixed_trust_fd() -> Result<File, RootEnrollmentError> {
    if rustix::process::geteuid().as_raw() != 0 {
        return Err(RootEnrollmentError::NotRoot);
    }
    let mut current = File::from(
        open(
            Path::new("/"),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(os)?,
    );
    if !root_dir_ok(&current.metadata()?, 0o755) {
        return Err(RootEnrollmentError::UntrustedPath);
    }
    for (name, mode) in [
        ("var", 0o755),
        ("lib", 0o755),
        ("akula", 0o700),
        ("server-bootstrap", 0o700),
        ("trust", 0o700),
    ] {
        let next = File::from(
            openat(
                &current,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(os)?,
        );
        if !root_dir_ok(&next.metadata()?, mode) {
            return Err(RootEnrollmentError::UntrustedPath);
        }
        current = next;
    }
    Ok(current)
}

pub(crate) mod bootstrap_helper;
pub(crate) use bootstrap_helper::inspect_provisioned_bootstrap_helper_hold;
mod owner_frame;
pub(crate) use owner_frame::load_verified_owner_frame;
mod enrollment;
mod owner_ipc;
pub use enrollment::admin_enroll_fixed_root_once;
pub use owner_ipc::admin_enroll_first_peer_uid_fixed_root_once;
mod witness_init;
pub use witness_init::admin_initialize_first_root_witness;
mod destination_init;
pub use destination_init::admin_enroll_fixed_opt_once;
mod release_pins;
pub use release_pins::{
    admin_enroll_first_b1a_fixed_root_once, admin_stage_first_b1a_fixed_root_once,
    open_provisioned_first_b1a_release, FirstReleaseError,
};

#[cfg(test)]
mod bootstrap_sequence_tests;
#[cfg(test)]
mod tests;
