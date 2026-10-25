//! Lab-only owner-signed mailbox policy, separate from Node-V1 Put/Get.
//!
//! This is NOT a production root policy loader: tests operate under a
//! process-owned private directory. A signed production mail service needs
//! independently root-owned file authority, a durable anti-rollback floor,
//! a trusted OS clock, protected custodian key enrollment, and a different
//! separately consented Node service activation/installer gate.
use super::MailRoleHold;
use akula_wire_lab::{
    MAX_MESSAGE_BYTES,
    custodian_mailbox::{MailRoleGate, MailboxPolicy},
    trust_w3::TrustRegistry,
};
use aquilum_core::node_v1::NodeHost;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rustix::fs::{Mode, OFlags, open, openat};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Component, Path, PathBuf},
};

const DOMAIN: &[u8] = b"Akula/NodeV1/WOW/OwnerMailRole/lab-v1\0";
const FILE: &str = "owner-mail-policy.bin";
const FLOOR_FILE: &str = "owner-mail-epoch-floor.bin";
const FLOOR_MAGIC: &[u8; 8] = b"WOWMLF01";
const FLOOR_LEN: usize = 8 + 8 + 32;
const MAX_FILE_BYTES: u64 = 4096;
const MAX_OFFERS: u32 = 64;
const PERMISSIONS: u8 = 0b0000_1111; // STORE/LIST/FETCH/RETURN
const DIR_MODE: u32 = 0o700;
const FILE_MODE: u32 = 0o600;
/// Production-path candidate, never chosen by a network request or CLI
/// argument. The /etc/akula-node ancestor is already independently
/// protected by existing Node-V1 RootTrust.
const ROOT_MAIL_PARENT: &str = "/etc/akula-node/wow-mail";
const ROOT_MAIL_DIR_MODE: u32 = 0o750;
const ROOT_MAIL_FILE_MODE: u32 = 0o640;

#[derive(Clone, Copy)]
enum PolicyFileAuthority {
    /// Legacy same-UID lab fixtures: unsafe for installed service.
    SameUidLab,
    /// Root:service-group read-only files, no service write authority.
    RootOwnedReadOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MailRoleClaims {
    pub version: u8,
    pub policy_epoch: u64,
    pub node_public: [u8; 32],
    pub node_endpoint: [u8; 32],
    /// Separately pinned by Akula Node RootTrust v3. Signature verification
    /// does not turn its pre-existing file grants into mail permissions.
    pub operator_public: [u8; 32],
    pub issuer: [u8; 16],
    pub issuer_epoch: u64,
    pub issuer_public: [u8; 32],
    pub custodian: [u8; 16],
    pub custodian_epoch: u64,
    pub custodian_public: [u8; 32],
    pub sender: [u8; 16],
    pub sender_epoch: u64,
    pub sender_public: [u8; 32],
    pub recipient: [u8; 16],
    pub recipient_epoch: u64,
    pub recipient_public: [u8; 32],
    pub mailbox: [u8; 16],
    pub scope: [u8; 16],
    pub permissions: u8,
    pub max_message_bytes: u32,
    pub max_offers: u32,
    pub valid_from_ms: i64,
    pub valid_until_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SignedPolicy {
    claims: MailRoleClaims,
    signature: Vec<u8>,
}

#[derive(Clone)]
pub(super) struct VerifiedMailRoleLab {
    pub(super) claims: MailRoleClaims,
    folder: PathBuf,
    inode: (u64, u64),
    pinned_digest: [u8; 32],
    floor_inode: (u64, u64),
    authority: PolicyFileAuthority,
}

pub fn sign_mail_policy_for_lab(signer: &SigningKey, claims: MailRoleClaims) -> Vec<u8> {
    let body = postcard::to_stdvec(&claims).expect("fixed-size fixture fields");
    let mut signed = Vec::with_capacity(DOMAIN.len() + body.len());
    signed.extend_from_slice(DOMAIN);
    signed.extend_from_slice(&body);
    let signature = signer.sign(&signed).to_bytes().to_vec();
    postcard::to_stdvec(&SignedPolicy { claims, signature }).expect("fixed-size policy + signature")
}

fn read_exact_private_file(
    folder: &Path,
    filename: &str,
    limit: u64,
) -> Result<(Vec<u8>, (u64, u64)), MailRoleHold> {
    let dir_meta = fs::symlink_metadata(folder).map_err(|_| MailRoleHold::MailPolicy)?;
    let uid = rustix::process::geteuid().as_raw();
    if !dir_meta.is_dir()
        || dir_meta.uid() != uid
        || dir_meta.permissions().mode() & 0o7777 != DIR_MODE
    {
        return Err(MailRoleHold::MailPolicy);
    }
    let dir = File::from(
        open(
            folder,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| MailRoleHold::MailPolicy)?,
    );
    let dir_opened = dir.metadata().map_err(|_| MailRoleHold::MailPolicy)?;
    if dir_meta.dev() != dir_opened.dev() || dir_meta.ino() != dir_opened.ino() {
        return Err(MailRoleHold::MailPolicy);
    }
    let handle = File::from(
        openat(
            &dir,
            filename,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| MailRoleHold::MailPolicy)?,
    );
    let metadata = handle.metadata().map_err(|_| MailRoleHold::MailPolicy)?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o7777 != FILE_MODE
        || metadata.len() == 0
        || metadata.len() > limit
    {
        return Err(MailRoleHold::MailPolicy);
    }
    let inode = (metadata.dev(), metadata.ino());
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    handle
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| MailRoleHold::MailPolicy)?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > limit {
        return Err(MailRoleHold::MailPolicy);
    }
    Ok((bytes, inode))
}

/// Read independently provisioned, root-owned PUBLIC policy bytes from
/// a non-root service account. Every path component is opened relative to
/// the previous verified directory FD with NOFOLLOW. Reject world/group
/// writable root ancestors, nonroot owners, and directory substitution.
/// This never reads secrets or writes /etc; only the privileged installer
/// is permitted to create or rotate these files.
fn read_root_owned_mail_file(
    folder: &Path,
    filename: &str,
    limit: u64,
) -> Result<(Vec<u8>, (u64, u64)), MailRoleHold> {
    if !folder.is_absolute() || rustix::process::geteuid().as_raw() == 0 {
        return Err(MailRoleHold::MailPolicy);
    }
    let mut dir = File::from(
        open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|_| MailRoleHold::MailPolicy)?,
    );
    let mut depth = 0;
    for component in folder.components() {
        match component {
            Component::RootDir => continue,
            Component::Normal(name) => {
                let next = File::from(
                    openat(
                        &dir,
                        Path::new(name),
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW
                            | OFlags::CLOEXEC,
                        Mode::empty(),
                    ).map_err(|_| MailRoleHold::MailPolicy)?,
                );
                let info = next.metadata().map_err(|_| MailRoleHold::MailPolicy)?;
                if !info.is_dir()
                    || info.uid() != 0
                    || info.permissions().mode() & 0o022 != 0
                {
                    return Err(MailRoleHold::MailPolicy);
                }
                depth += 1;
                dir = next;
            }
            _ => return Err(MailRoleHold::MailPolicy),
        }
    }
    // "/" or "/etc" are never mailbox authority directories. The final
    // directory must be root-owned and accessible only by service group.
    let folder_info = dir.metadata().map_err(|_| MailRoleHold::MailPolicy)?;
    if depth < 2
        || folder_info.gid() != rustix::process::getegid().as_raw()
        || folder_info.permissions().mode() & 0o7777 != ROOT_MAIL_DIR_MODE
    {
        return Err(MailRoleHold::MailPolicy);
    }
    let handle = File::from(
        openat(
            &dir,
            filename,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|_| MailRoleHold::MailPolicy)?,
    );
    let meta = handle.metadata().map_err(|_| MailRoleHold::MailPolicy)?;
    if !meta.is_file()
        || meta.uid() != 0
        || meta.gid() != rustix::process::getegid().as_raw()
        || meta.nlink() != 1
        || meta.permissions().mode() & 0o7777 != ROOT_MAIL_FILE_MODE
        || meta.len() == 0
        || meta.len() > limit
    {
        return Err(MailRoleHold::MailPolicy);
    }
    let mut result = Vec::with_capacity(meta.len() as usize);
    handle.take(limit + 1).read_to_end(&mut result)
        .map_err(|_| MailRoleHold::MailPolicy)?;
    if result.len() as u64 != meta.len() {
        return Err(MailRoleHold::MailPolicy);
    }
    Ok((result, (meta.dev(), meta.ino())))
}

fn read_authorized_mail_file(
    folder: &Path,
    filename: &str,
    limit: u64,
    authority: PolicyFileAuthority,
) -> Result<(Vec<u8>, (u64, u64)), MailRoleHold> {
    match authority {
        PolicyFileAuthority::SameUidLab => read_exact_private_file(folder, filename, limit),
        PolicyFileAuthority::RootOwnedReadOnly => read_root_owned_mail_file(folder, filename, limit),
    }
}

/// Lab provisioning fixture only. The floor MUST be protected by a
/// separate root-controlled anti-rollback authority in production. A
/// same-UID attacker could replace BOTH lab files; the test never claims
/// otherwise. Never self-increment this file in the running service.
pub fn mail_epoch_floor_for_lab(signed_policy: &[u8], epoch: u64) -> Vec<u8> {
    let mut result = Vec::with_capacity(FLOOR_LEN);
    result.extend_from_slice(FLOOR_MAGIC);
    result.extend_from_slice(&epoch.to_be_bytes());
    result.extend_from_slice(blake3::hash(signed_policy).as_bytes());
    result
}

struct EpochFloor {
    epoch: u64,
    digest: [u8; 32],
    inode: (u64, u64),
}

fn read_epoch_floor(folder: &Path, authority: PolicyFileAuthority) -> Result<EpochFloor, MailRoleHold> {
    let (bytes, inode) = read_authorized_mail_file(folder, FLOOR_FILE, FLOOR_LEN as u64, authority)?;
    if bytes.len() != FLOOR_LEN || &bytes[..8] != FLOOR_MAGIC {
        return Err(MailRoleHold::MailPolicy);
    }
    let epoch = u64::from_be_bytes(
        bytes[8..16]
            .try_into()
            .map_err(|_| MailRoleHold::MailPolicy)?,
    );
    let digest: [u8; 32] = bytes[16..]
        .try_into()
        .map_err(|_| MailRoleHold::MailPolicy)?;
    if epoch == 0 || digest == [0; 32] {
        return Err(MailRoleHold::MailPolicy);
    }
    Ok(EpochFloor {
        epoch,
        digest,
        inode,
    })
}

fn decode_and_verify(
    input: &[u8],
    node: &NodeHost,
    expected_endpoint: [u8; 32],
    now_ms: i64,
) -> Result<MailRoleClaims, MailRoleHold> {
    let signed: SignedPolicy = postcard::from_bytes(input).map_err(|_| MailRoleHold::MailPolicy)?;
    // Reject alternate encodings and trailing/unknown data.
    if postcard::to_stdvec(&signed).map_err(|_| MailRoleHold::MailPolicy)? != input {
        return Err(MailRoleHold::MailPolicy);
    }
    let p = signed.claims;
    if p.version != 1
        || p.policy_epoch == 0
        || p.node_public != node.public_key()
        || p.node_endpoint != expected_endpoint
        || expected_endpoint == [0; 32]
        || p.operator_public == [0; 32]
        || p.operator_public
            != node
                .mail_operator_key_for_lab()
                .map_err(|_| MailRoleHold::NodePolicy)?
        || p.issuer == [0; 16]
        || p.issuer_epoch == 0
        || p.custodian == [0; 16]
        || p.custodian_epoch == 0
        || p.sender == [0; 16]
        || p.sender_epoch == 0
        || p.recipient == [0; 16]
        || p.recipient_epoch == 0
        || p.issuer_public == [0; 32]
        || p.custodian_public == [0; 32]
        || p.sender_public == [0; 32]
        || p.recipient_public == [0; 32]
        || p.mailbox == [0; 16]
        || p.scope == [0; 16]
        || p.permissions != PERMISSIONS
        || p.max_message_bytes == 0
        || p.max_message_bytes as usize > MAX_MESSAGE_BYTES
        || p.max_offers == 0
        || p.max_offers > MAX_OFFERS
        || p.valid_from_ms < 0
        || p.valid_until_ms <= p.valid_from_ms
        || now_ms < p.valid_from_ms
        || now_ms >= p.valid_until_ms
        || p.issuer_epoch > i64::MAX as u64
        || p.custodian_epoch > i64::MAX as u64
        || p.sender_epoch > i64::MAX as u64
        || p.recipient_epoch > i64::MAX as u64
    {
        return Err(MailRoleHold::MailPolicy);
    }
    // A single allowed signer cannot simultaneously be a sender, recipient
    // or custodian under another ID and bypass domain-separated access.
    let ids = [p.issuer, p.custodian, p.sender, p.recipient];
    for (i, identity) in ids.iter().enumerate() {
        if ids[i + 1..].contains(identity) {
            return Err(MailRoleHold::MailPolicy);
        }
    }
    node.verify_iroh_endpoint(expected_endpoint)
        .map_err(|_| MailRoleHold::NodePolicy)?;
    let verifying =
        VerifyingKey::from_bytes(&p.operator_public).map_err(|_| MailRoleHold::MailPolicy)?;
    let body = postcard::to_stdvec(&p).map_err(|_| MailRoleHold::MailPolicy)?;
    let mut message = Vec::with_capacity(DOMAIN.len() + body.len());
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(&body);
    let signature =
        Signature::from_slice(&signed.signature).map_err(|_| MailRoleHold::MailPolicy)?;
    verifying
        .verify(&message, &signature)
        .map_err(|_| MailRoleHold::MailPolicy)?;
    Ok(p)
}

impl VerifiedMailRoleLab {
    pub(super) fn open(
        folder: &Path,
        node: &NodeHost,
        expected_endpoint: [u8; 32],
        now_ms: i64,
    ) -> Result<Self, MailRoleHold> {
        Self::open_with_authority(
            folder, node, expected_endpoint, now_ms, PolicyFileAuthority::SameUidLab,
        )
    }

    /// A separate fixed service policy path, root:service-group protected,
    /// readable to nonroot Node but never writable by it. Does NOT activate
    /// any listener, installer or mail role without an existing SQLite gate.
    pub(super) fn open_root_provisioned(
        node: &NodeHost,
        expected_endpoint: [u8; 32],
        now_ms: i64,
    ) -> Result<Self, MailRoleHold> {
        Self::open_with_authority(
            Path::new(ROOT_MAIL_PARENT), node, expected_endpoint, now_ms,
            PolicyFileAuthority::RootOwnedReadOnly,
        )
    }

    fn open_with_authority(
        folder: &Path,
        node: &NodeHost,
        expected_endpoint: [u8; 32],
        now_ms: i64,
        authority: PolicyFileAuthority,
    ) -> Result<Self, MailRoleHold> {
        let (input, inode) = read_authorized_mail_file(folder, FILE, MAX_FILE_BYTES, authority)?;
        let claims = decode_and_verify(&input, node, expected_endpoint, now_ms)?;
        let floor = read_epoch_floor(folder, authority)?;
        let digest = *blake3::hash(&input).as_bytes();
        if floor.epoch != claims.policy_epoch || floor.digest != digest {
            return Err(MailRoleHold::MailPolicy);
        }
        Ok(Self {
            claims,
            folder: folder.to_path_buf(),
            inode,
            pinned_digest: digest,
            floor_inode: floor.inode,
            authority,
        })
    }

    /// Reject in-place replacement, byte alteration, revocation, expiry
    /// and node policy change on EVERY request. A new policy requires an
    /// explicit reviewed restart; never hot-adopt a newer/malicious file.
    pub(super) fn require_current(&self, node: &NodeHost, now_ms: i64) -> Result<(), MailRoleHold> {
        let (input, inode) = read_authorized_mail_file(
            &self.folder, FILE, MAX_FILE_BYTES, self.authority,
        )?;
        let floor = read_epoch_floor(&self.folder, self.authority)?;
        if inode != self.inode
            || *blake3::hash(&input).as_bytes() != self.pinned_digest
            || floor.inode != self.floor_inode
            || floor.epoch != self.claims.policy_epoch
            || floor.digest != self.pinned_digest
        {
            return Err(MailRoleHold::MailPolicy);
        }
        if decode_and_verify(&input, node, self.claims.node_endpoint, now_ms)? != self.claims {
            return Err(MailRoleHold::MailPolicy);
        }
        Ok(())
    }

    /// The signed manifest was authenticated through the current Node
    /// root-pinned operator key and independent epoch floor. Bind its exact
    /// digest and mailbox limits to WOW's SQLite transaction-time guard.
    pub(super) fn sqlite_gate(&self) -> MailRoleGate {
        let p = self.claims;
        MailRoleGate {
            mailbox_policy: MailboxPolicy {
                mailbox: p.mailbox,
                issuer: p.issuer,
                issuer_epoch: p.issuer_epoch,
                custodian: p.custodian,
                scope: p.scope,
                recipient: p.recipient,
                max_message_bytes: p.max_message_bytes,
                max_offers: p.max_offers,
            },
            policy_epoch: p.policy_epoch,
            signed_policy_digest: self.pinned_digest,
        }
    }

    pub(super) fn registry(&self) -> Result<TrustRegistry, MailRoleHold> {
        let p = self.claims;
        let mut registry = TrustRegistry::new();
        for (identity, epoch, key) in [
            (p.issuer, p.issuer_epoch, p.issuer_public),
            (p.custodian, p.custodian_epoch, p.custodian_public),
            (p.sender, p.sender_epoch, p.sender_public),
            (p.recipient, p.recipient_epoch, p.recipient_public),
        ] {
            registry
                .enroll(
                    identity,
                    epoch,
                    VerifyingKey::from_bytes(&key).map_err(|_| MailRoleHold::MailPolicy)?,
                )
                .map_err(|_| MailRoleHold::MailPolicy)?;
        }
        Ok(registry)
    }
}
