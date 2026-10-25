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
use akula_wire_lab::{
    custodian_wire::{self, WireError},
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
    custodian_id: [u8; 16],
    custodian_epoch: u64,
    expected_node_endpoint: [u8; 32],
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
    /// Open only the exact already provisioned local SQLite. The default
    /// Akula Node role NEVER calls this; the test feature must be explicit.
    /// TrustRegistry and signing secret are externally provisioned lab inputs.
    /// In production these need root-pinned owner mailbox policy and an
    /// independently enrolled key store, neither of which exists in v3.
    pub fn open_existing(
        node: &NodeHost,
        state: &Path,
        node_endpoint: [u8; 32],
        custodian_id: [u8; 16],
        custodian_epoch: u64,
        custodian_signer: SigningKey,
        trust: TrustRegistry,
    ) -> Result<Self, MailRoleHold> {
        if node_endpoint == [0; 32] {
            return Err(MailRoleHold::InvalidIdentity);
        }
        node.verify_iroh_endpoint(node_endpoint)
            .map_err(|_| MailRoleHold::NodePolicy)?;
        if custodian_id == [0; 16] || custodian_epoch == 0 || custodian_epoch > i64::MAX as u64 {
            return Err(MailRoleHold::InvalidIdentity);
        }
        trust
            .verify(
                custodian_id,
                custodian_epoch,
                &custodian_signer.verifying_key(),
            )
            .map_err(|_| MailRoleHold::UnpinnedTrust)?;
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
            custodian_id,
            custodian_epoch,
            expected_node_endpoint: node_endpoint,
            trust,
            last_now_ms: 0,
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
        node.verify_iroh_endpoint(self.expected_node_endpoint)
            .map_err(|_| MailRoleHold::NodePolicy)?;
        self.trust
            .verify(
                self.custodian_id,
                self.custodian_epoch,
                &self.custodian_signer.verifying_key(),
            )
            .map_err(|_| MailRoleHold::UnpinnedTrust)?;
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
        custodian_wire::serve_one_loopback(
            listener,
            &mut self.journal,
            &self.custodian_signer,
            self.custodian_epoch,
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
