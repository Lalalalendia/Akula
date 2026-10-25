//! Opt-in NON-SHIPPING WOW mailbox bridge for Akula Node-V1.
//!
//! The existing Node-V1 signed installer, /opt ELF, service ALPN and root
//! policy v3 authorize file Put/Get only. They DO NOT authorize WOW mail.
//! This bridge reuses a pre-provisioned WOW journal and separate signatures,
//! and refuses every request if the Node-V1 Iroh identity/root-policy pin
//! has changed. There is intentionally NO public Iroh handler, daemon mode,
//! installer activation, key enrollment or automatic mailbox creation.
//!
//! A future product role requires an independently owner-approved mail policy
//! pinned by root before any network exposure. This feature is lab-only.
mod policy;
pub use policy::{MailRoleClaims, sign_mail_policy_for_lab};
use policy::VerifiedMailRoleLab;
use akula_wire_lab::{
    custodian_wire,
    journal_w4::Journal,
    trust_w3::TrustRegistry,
};
use aquilum_core::node_v1::NodeHost;
use ed25519_dalek::SigningKey;
use std::{
    fs::{self, Metadata},
    net::TcpListener,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

const DB: &str = "wow-mailbox.sqlite";
const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;

#[derive(Debug)]
pub enum MailRoleHold {
    NodePolicy,
    InvalidIdentity,
    UnpinnedTrust,
    MailPolicy,
    UnsafeState,
    UnsafeDatabase,
    ClockRollback,
    Journal,
    Wire,
}

/// Separate non-shipping mailbox authority. An in-memory TrustRegistry
/// supplied by a local lab is NOT proof of owner/root provisioning. The only
/// network entry is the existing signed WOW localhost protocol.
pub struct NodeWowMailboxLab {
    journal: Journal,
    state: PathBuf,
    db_inode: (u64, u64),
    custodian_signer: SigningKey,
    policy: VerifiedMailRoleLab,
    trust: TrustRegistry,
    last_now_ms: i64,
}

fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn private_dir(dir: &Path) -> Result<Metadata, MailRoleHold> {
    let info = fs::symlink_metadata(dir).map_err(|_| MailRoleHold::UnsafeState)?;
    if !info.is_dir()
        || info.uid() != current_uid()
        || info.permissions().mode() & 0o7777 != PRIVATE_DIR_MODE
    {
        return Err(MailRoleHold::UnsafeState);
    }
    Ok(info)
}

fn private_existing_db(state: &Path) -> Result<Metadata, MailRoleHold> {
    private_dir(state)?;
    let info = fs::symlink_metadata(state.join(DB)).map_err(|_| MailRoleHold::UnsafeDatabase)?;
    if !info.is_file()
        || info.uid() != current_uid()
        || info.nlink() != 1
        || info.len() == 0
        || info.permissions().mode() & 0o7777 != PRIVATE_FILE_MODE
    {
        return Err(MailRoleHold::UnsafeDatabase);
    }
    Ok(info)
}

impl NodeWowMailboxLab {
    /// Open an already provisioned mailbox with a separately signed policy
    /// anchored to Node's *independently* pinned operator key (read-only).
    /// No caller-supplied TrustRegistry or mailbox identities are trusted.
    /// Test-only: parent file ownership here is same-UID, NOT production root.
    pub fn open_existing(
        node: &NodeHost,
        state: &Path,
        mail_policy_folder: &Path,
        node_endpoint: [u8; 32],
        custodian_signer: SigningKey,
        now_ms: i64,
    ) -> Result<Self, MailRoleHold> {
        let policy = VerifiedMailRoleLab::open(
            mail_policy_folder, node, node_endpoint, now_ms,
        )?;
        let claims = policy.claims;
        if claims.custodian_public != custodian_signer.verifying_key().to_bytes() {
            return Err(MailRoleHold::UnpinnedTrust);
        }
        let trust = policy.registry()?;
        let db_info = private_existing_db(state)?;
        let db_path = state.join(DB);
        let journal = Journal::open(&db_path).map_err(|_| MailRoleHold::Journal)?;
        let reopened = private_existing_db(state)?;
        if (db_info.dev(), db_info.ino()) != (reopened.dev(), reopened.ino()) {
            return Err(MailRoleHold::UnsafeDatabase);
        }
        Ok(Self {
            journal,
            state: state.to_path_buf(),
            db_inode: (db_info.dev(), db_info.ino()),
            custodian_signer,
            policy,
            trust,
            last_now_ms: now_ms,
        })
    }

    /// Bound one request to the independently root-pinned *Node transport*
    /// identity AND to the distinct enrolled WOW custodian identity.
    /// These checks are necessary, NOT sufficient for production mailbox
    /// activation; a separate root/owner mail-policy gate is still required.
    fn require_current(&self, node: &NodeHost, now_ms: i64) -> Result<(), MailRoleHold> {
        if now_ms <= 0 || now_ms < self.last_now_ms {
            return Err(MailRoleHold::ClockRollback);
        }
        self.policy.require_current(node, now_ms)?;
        if self.policy.claims.custodian_public
            != self.custodian_signer.verifying_key().to_bytes()
        {
            return Err(MailRoleHold::UnpinnedTrust);
        }
        let info = private_existing_db(&self.state)?;
        if (info.dev(), info.ino()) != self.db_inode {
            return Err(MailRoleHold::UnsafeDatabase);
        }
        Ok(())
    }

    /// Serve one bounded localhost connection using the already audited WOW
    /// STORE/LIST/FETCH/opaque-W4 code. No new public transport or API is
    /// created. A failed request cannot authorize a File Gate operation.
    pub fn serve_one(
        &mut self,
        node: &NodeHost,
        listener: &TcpListener,
        now_ms: i64,
    ) -> Result<(), MailRoleHold> {
        self.require_current(node, now_ms)?;
        Self::ensure_local_only(listener)?;
        custodian_wire::serve_one_loopback(
            listener,
            &mut self.journal,
            &self.custodian_signer,
            self.policy.claims.custodian_epoch,
            &self.trust,
            now_ms,
        )
        .map_err(|_| MailRoleHold::Wire)?;
        self.last_now_ms = now_ms;
        Ok(())
    }

    /// An unsafe input cannot be treated as a recoverable network denial.
    /// Reopen requires the same explicit trust/Node-policy preflight again.
    pub fn ensure_local_only(listener: &TcpListener) -> Result<(), MailRoleHold> {
        if listener.local_addr().map_err(|_| MailRoleHold::Wire)?.ip()
            == std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        {
            Ok(())
        } else {
            Err(MailRoleHold::Wire)
        }
    }
}
