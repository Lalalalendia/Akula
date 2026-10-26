//! Non-shipping root-only admission of the EXACT first-root staged helper.
//! This validates the Python root_stage_v1.py published bootstrap-v1 HOLD
//! against the independently enrolled B0v3 HELPER SHA-256. The bootstrap
//! publisher key present in the stage is NOT itself an independent pin:
//! the compiled Windows production key is still deliberately unprovisioned.
//! No executable FD is returned, invoked, installed, or activated.
use super::*;
use crate::server_installer_pinned_release_lab::decode_b1a_wire;
use crate::server_release_lab::{
    verify_signed_release, ReleaseFloor, SignedRelease, TrustedReleaseSigner,
};
use ring::digest::{Context, SHA256};
use rustix::fs::{openat, Mode, OFlags};
use std::os::unix::fs::FileExt;
use std::time::{SystemTime, UNIX_EPOCH};

const STAGE_LEAF: &str = "bootstrap-v1";
const HELPER_LEAF: &str = "helper.staged";
const HELPER_SIGNER: &str = "akula-root-bootstrap-v1";
const HOLD_DOMAIN: &[u8] = b"AKULA_ROOT_BOOTSTRAP_SIGNED_STAGE_ONLY_V1\n";
const MAX_HELPER_BYTES: u64 = 64 * 1024 * 1024;
const SPKI_PREFIX: &[u8] = b"\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00";

#[derive(Debug)]
pub(crate) enum BootstrapHelperError {
    Trust(RootEnrollmentError),
    InvalidStage,
    InvalidSignature,
    ChangedInode,
    Io(std::io::Error),
}
impl From<RootEnrollmentError> for BootstrapHelperError {
    fn from(value: RootEnrollmentError) -> Self {
        Self::Trust(value)
    }
}
impl From<std::io::Error> for BootstrapHelperError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
fn syscall(value: rustix::io::Errno) -> BootstrapHelperError {
    BootstrapHelperError::Io(value.into())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BootstrapHelperHold {
    pub sha256: [u8; 32],
    pub size: u64,
    pub release_sequence: u64,
    /// No FD and no installation/activation capability escapes this proof.
    pub remains_hold: bool,
}

fn protected(dir: &File, name: &str, maximum: u64) -> Result<Vec<u8>, BootstrapHelperError> {
    let file = File::from(
        openat(
            dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    let before = file.metadata()?;
    if !root_file_ok(&before, 0o400, maximum) || before.len() == 0 {
        return Err(BootstrapHelperError::InvalidStage);
    }
    let mut bytes = Vec::new();
    (&file).take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 != before.len() || !stable(&before, &file.metadata()?) {
        return Err(BootstrapHelperError::ChangedInode);
    }
    Ok(bytes)
}

/// Exact public key PEM stored by root_stage_v1.py. The PEM must not carry
/// a second key or a different verifier identity from release.ed25519.
pub(super) fn expected_pub_pem(key: &[u8; 32]) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut der = Vec::with_capacity(SPKI_PREFIX.len() + key.len());
    der.extend_from_slice(SPKI_PREFIX);
    der.extend_from_slice(key);
    let mut pem = b"-----BEGIN PUBLIC KEY-----\n".to_vec();
    for chunk in der.chunks(3) {
        let first = chunk[0];
        let second = *chunk.get(1).unwrap_or(&0);
        let third = *chunk.get(2).unwrap_or(&0);
        pem.push(TABLE[(first >> 2) as usize]);
        pem.push(TABLE[(((first & 3) << 4) | (second >> 4)) as usize]);
        pem.push(if chunk.len() >= 2 {
            TABLE[(((second & 15) << 2) | (third >> 6)) as usize]
        } else {
            b'='
        });
        pem.push(if chunk.len() >= 3 {
            TABLE[(third & 63) as usize]
        } else {
            b'='
        });
    }
    pem.extend_from_slice(b"\n-----END PUBLIC KEY-----\n");
    pem
}

/// Read-only root FD primitive. A valid signature from the staged public key
/// is necessary but NOT sufficient: only the separately enrolled B0v3 policy
/// supplies the expected digest of the exact helper bytes. This prevents
/// self-signed, attacker-selected stage bytes from granting any authority.
pub(super) fn inspect_from_trust_fd(
    trust: &File,
    now_unix: u64,
) -> Result<BootstrapHelperHold, BootstrapHelperError> {
    let policy = RootEnrolledV3::from_trust_fd(trust)?;
    let parent = File::from(
        openat(
            trust,
            "..",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&parent.metadata()?, 0o700) {
        return Err(BootstrapHelperError::InvalidStage);
    }
    let stage = File::from(
        openat(
            &parent,
            STAGE_LEAF,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    if !root_dir_ok(&stage.metadata()?, 0o700) {
        return Err(BootstrapHelperError::InvalidStage);
    }

    let raw_manifest = protected(&stage, "manifest", 256)?;
    let manifest =
        decode_b1a_wire(&raw_manifest).map_err(|_| BootstrapHelperError::InvalidStage)?;
    if manifest.signer_id != HELPER_SIGNER
        || manifest.package_sha256 != policy.helper_sha256
        || manifest.package_size < 64
        || manifest.package_size > MAX_HELPER_BYTES
    {
        return Err(BootstrapHelperError::InvalidStage);
    }
    let signer_key: [u8; 32] = protected(&stage, "release.ed25519", 32)?
        .try_into()
        .map_err(|_| BootstrapHelperError::InvalidStage)?;
    if signer_key == [0; 32]
        || protected(&stage, "release.pub", 1024)? != expected_pub_pem(&signer_key)
    {
        return Err(BootstrapHelperError::InvalidStage);
    }
    let signature: [u8; 64] = protected(&stage, "signature", 64)?
        .try_into()
        .map_err(|_| BootstrapHelperError::InvalidStage)?;
    verify_signed_release(
        &SignedRelease {
            manifest: manifest.clone(),
            signature,
        },
        &TrustedReleaseSigner {
            signer_id: HELPER_SIGNER.to_owned(),
            ed25519_public: signer_key,
        },
        ReleaseFloor::initial(),
        now_unix,
    )
    .map_err(|_| BootstrapHelperError::InvalidSignature)?;

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut marker = HOLD_DOMAIN.to_vec();
    for b in manifest.package_sha256 {
        marker.push(HEX[(b >> 4) as usize]);
        marker.push(HEX[(b & 15) as usize]);
    }
    marker.push(b'\n');
    if protected(&stage, "HOLD", 256)? != marker {
        return Err(BootstrapHelperError::InvalidStage);
    }

    let file = File::from(
        openat(
            &stage,
            HELPER_LEAF,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(syscall)?,
    );
    let before = file.metadata()?;
    if !root_file_ok(&before, 0o400, MAX_HELPER_BYTES) || before.len() != manifest.package_size {
        return Err(BootstrapHelperError::InvalidStage);
    }
    let mut hash = Context::new(&SHA256);
    let mut head = [0u8; 20];
    let mut offset = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    while offset < before.len() {
        let remaining = before.len() - offset;
        let want = buffer.len().min(remaining as usize);
        let read = file.read_at(&mut buffer[..want], offset)?;
        if read == 0 {
            return Err(BootstrapHelperError::ChangedInode);
        }
        let head_count = ((head.len() as u64).saturating_sub(offset)) as usize;
        let copy = head_count.min(read);
        if copy != 0 {
            head[offset as usize..offset as usize + copy].copy_from_slice(&buffer[..copy]);
        }
        hash.update(&buffer[..read]);
        offset += read as u64;
    }
    if head[..7] != b"\x7fELF\x02\x01\x01"[..]
        || head[18..20] != b"\x3e\x00"[..]
        || hash.finish().as_ref() != manifest.package_sha256.as_slice()
        || !stable(&before, &file.metadata()?)
    {
        return Err(BootstrapHelperError::InvalidStage);
    }
    Ok(BootstrapHelperHold {
        sha256: manifest.package_sha256,
        size: before.len(),
        release_sequence: manifest.release_sequence,
        remains_hold: true,
    })
}

/// Root-only fixed-path readback. No client-selected signer, digest, clock,
/// staging path, helper FD, process spawn, root IPC opcode or systemd.
#[allow(dead_code)]
pub(crate) fn inspect_provisioned_bootstrap_helper_hold(
) -> Result<BootstrapHelperHold, BootstrapHelperError> {
    let trust = open_fixed_trust_fd()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BootstrapHelperError::InvalidStage)?
        .as_secs();
    inspect_from_trust_fd(&trust, now)
}

/// Independent positive admission of the actual running root-helper image.
/// A signed, staged package alone is insufficient; compare the executable
/// inode of the running process with an exact fixed root-owned install.
pub(crate) fn inspect_provisioned_running_helper_hold(
) -> Result<BootstrapHelperHold, BootstrapHelperError> {
    let trust = open_fixed_trust_fd()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| BootstrapHelperError::InvalidStage)?
        .as_secs();
    let approved = inspect_from_trust_fd(&trust, now)?;
    running::verify_installed_self_image(&approved)
        .map_err(|_| BootstrapHelperError::InvalidStage)?;
    Ok(approved)
}

mod running;

#[cfg(test)]
mod tests;
