//! Strict bounded B1b1b owner-frame decoder (never installation authority).
//! The same canonical Ed25519 bytes are shared by Windows and Linux.
use super::*;

const FLOOR_BYTES: usize = 8 + 8 + (3 * 4) + 32 + 32;
const FIXED_BYTES: usize = 16 + (3 * 32) + (2 * FLOOR_BYTES) + 32 + 8 + 8 + 16 + 8 + 8;
const MAX_ID_BYTES: usize = 64;
const MAX_CANONICAL_BYTES: usize = DOMAIN.len() + 2 + MAX_ID_BYTES + FIXED_BYTES;

fn take<'a>(input: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], WireError> {
    let end = cursor.checked_add(len).ok_or(WireError::MalformedClaim)?;
    let bytes = input.get(*cursor..end).ok_or(WireError::MalformedClaim)?;
    *cursor = end;
    Ok(bytes)
}
fn field<const N: usize>(input: &[u8], cursor: &mut usize) -> Result<[u8; N], WireError> {
    take(input, cursor, N)?
        .try_into()
        .map_err(|_| WireError::MalformedClaim)
}
fn take_u64(input: &[u8], cursor: &mut usize) -> Result<u64, WireError> {
    Ok(u64::from_be_bytes(field::<8>(input, cursor)?))
}
fn take_u32(input: &[u8], cursor: &mut usize) -> Result<u32, WireError> {
    Ok(u32::from_be_bytes(field::<4>(input, cursor)?))
}
fn take_floor(input: &[u8], cursor: &mut usize) -> Result<OwnerFloorWire, WireError> {
    let generation = take_u64(input, cursor)?;
    let sequence = take_u64(input, cursor)?;
    let version = [
        take_u32(input, cursor)?,
        take_u32(input, cursor)?,
        take_u32(input, cursor)?,
    ];
    let manifest_fingerprint = field::<32>(input, cursor)?;
    let chain_tip = field::<32>(input, cursor)?;
    Ok(OwnerFloorWire {
        generation,
        sequence,
        version,
        manifest_fingerprint,
        chain_tip,
    })
}

impl OwnerApprovalWireV1 {
    /// Parse only the existing canonical signed bytes. Pure data; no trust pin
    /// or privileged action can be supplied by the incoming frame.
    pub fn decode_canonical_bytes(input: &[u8]) -> Result<Self, WireError> {
        if input.len() > MAX_CANONICAL_BYTES || !input.starts_with(DOMAIN) {
            return Err(WireError::MalformedClaim);
        }
        let mut cursor = DOMAIN.len();
        if field::<1>(input, &mut cursor)? != [1] {
            return Err(WireError::MalformedClaim);
        }
        let id_len = field::<1>(input, &mut cursor)?[0] as usize;
        if !(1..=MAX_ID_BYTES).contains(&id_len)
            || input.len() != DOMAIN.len() + 2 + id_len + FIXED_BYTES
        {
            return Err(WireError::MalformedClaim);
        }
        let approver_id = std::str::from_utf8(take(input, &mut cursor, id_len)?)
            .map_err(|_| WireError::MalformedClaim)?
            .to_owned();
        let install_id = field::<16>(input, &mut cursor)?;
        let plan_digest = field::<32>(input, &mut cursor)?;
        let ssh_host_sha256 = field::<32>(input, &mut cursor)?;
        let manifest_fingerprint = field::<32>(input, &mut cursor)?;
        let previous = take_floor(input, &mut cursor)?;
        let proposed = take_floor(input, &mut cursor)?;
        let staged_package_sha256 = field::<32>(input, &mut cursor)?;
        let staged_package_size = take_u64(input, &mut cursor)?;
        let expected_witness_revision = take_u64(input, &mut cursor)?;
        let approval_nonce = field::<16>(input, &mut cursor)?;
        let issued_at_unix = take_u64(input, &mut cursor)?;
        let expires_at_unix = take_u64(input, &mut cursor)?;
        let decoded = Self {
            approver_id,
            install_id,
            plan_digest,
            ssh_host_sha256,
            manifest_fingerprint,
            previous,
            proposed,
            staged_package_sha256,
            staged_package_size,
            expected_witness_revision,
            approval_nonce,
            issued_at_unix,
            expires_at_unix,
        };
        // Syntax/shape is checked without accepting a caller-selected clock.
        // Expired claims can be decoded for audit but never verified as fresh.
        decoded.validate_time_and_shape(issued_at_unix)?;
        if cursor != input.len() || decoded.canonical_bytes() != input {
            return Err(WireError::MalformedClaim);
        }
        Ok(decoded)
    }
}

/// Canonical owner claim followed by EXACTLY 64 Ed25519 signature bytes.
/// Parsing and checking a frame does not authorize installation or CAS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedOwnerWireFrame {
    pub claim: OwnerApprovalWireV1,
    pub signature: [u8; 64],
}
impl SignedOwnerWireFrame {
    pub fn decode(input: &[u8]) -> Result<Self, WireError> {
        if input.len() <= 64 || input.len() > MAX_CANONICAL_BYTES + 64 {
            return Err(WireError::MalformedClaim);
        }
        let (claim, signature) = input.split_at(input.len() - 64);
        Ok(Self {
            claim: OwnerApprovalWireV1::decode_canonical_bytes(claim)?,
            signature: signature
                .try_into()
                .map_err(|_| WireError::MalformedClaim)?,
        })
    }

    pub fn canonical_frame(&self) -> Vec<u8> {
        let mut bytes = self.claim.canonical_bytes();
        bytes.extend_from_slice(&self.signature);
        bytes
    }

    /// Caller must obtain the expected identity and public key from an
    /// independently enrolled root policy, NOT SSH/package/desktop payload.
    pub fn verify_pinned_owner(
        &self,
        independently_enrolled_owner_id: &str,
        independently_enrolled_owner_key: &[u8; 32],
        now_unix: u64,
    ) -> Result<(), WireError> {
        if self.claim.approver_id != independently_enrolled_owner_id {
            return Err(WireError::UntrustedOwner);
        }
        self.claim
            .verify_detached(&self.signature, independently_enrolled_owner_key, now_unix)
    }

    /// Pure conversion to the existing Linux B1b1b CAS input, with NO grant.
    #[cfg(all(target_os = "linux", feature = "server-bootstrap-b1b1b-approval-lab"))]
    pub fn into_inert_linux_claim(self) -> crate::server_owner_approval_lab::SignedOwnerApproval {
        use crate::server_owner_approval_lab::{SignedApprovalClaim, SignedOwnerApproval};
        use crate::server_release_floor_lab::{LabPlanBinding, TrustedFloorHead};
        use crate::server_release_lab::{ReleaseFloor, Version};
        use crate::server_staging_lab::{ApprovedPlanDigest, InstallId};

        let head = |h: OwnerFloorWire| TrustedFloorHead {
            generation: h.generation,
            floor: ReleaseFloor {
                sequence: h.sequence,
                version: Version {
                    major: h.version[0],
                    minor: h.version[1],
                    patch: h.version[2],
                },
                signed_manifest_fingerprint: h.manifest_fingerprint,
            },
            chain_tip: h.chain_tip,
        };
        SignedOwnerApproval {
            claim: SignedApprovalClaim {
                approver_id: self.claim.approver_id,
                scope_binding: LabPlanBinding {
                    install_id: InstallId(self.claim.install_id),
                    plan_digest: ApprovedPlanDigest(self.claim.plan_digest),
                    pinned_host_key_sha256: self.claim.ssh_host_sha256,
                    signed_manifest_fingerprint: self.claim.manifest_fingerprint,
                },
                previous_head: head(self.claim.previous),
                proposed_head: head(self.claim.proposed),
                staged_package_sha256: self.claim.staged_package_sha256,
                staged_package_size: self.claim.staged_package_size,
                expected_witness_revision: self.claim.expected_witness_revision,
                approval_nonce: self.claim.approval_nonce,
                issued_at_unix: self.claim.issued_at_unix,
                expires_at_unix: self.claim.expires_at_unix,
            },
            signature: self.signature,
        }
    }
}
#[cfg(test)]
mod tests;
