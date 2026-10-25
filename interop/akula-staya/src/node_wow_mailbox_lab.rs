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
use akula_wire_lab::{custodian_wire, journal_w4::Journal, trust_w3::TrustRegistry};
use aquilum_core::node_v1::NodeHost;
use ed25519_dalek::SigningKey;
use policy::VerifiedMailRoleLab;
pub use policy::{MailRoleClaims, mail_epoch_floor_for_lab, sign_mail_policy_for_lab};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
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

/// Fail closed if the durable WOW schema grants a different mailbox,
/// broader quota, different owner epoch, or other identities than the
/// separately signed owner policy. This is a lab preflight: real product
/// admission must check the policy INSIDE the same SQLite write transaction.
fn require_exact_mailbox(state: &Path, policy: &VerifiedMailRoleLab) -> Result<(), MailRoleHold> {
    let p = policy.claims;
    let db = Connection::open_with_flags(
        state.join(DB),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| MailRoleHold::MailPolicy)?;
    let total: i64 = db
        .query_row("SELECT count(*) FROM custodian_mailbox_policy", [], |r| {
            r.get(0)
        })
        .map_err(|_| MailRoleHold::MailPolicy)?;
    if total != 1 {
        return Err(MailRoleHold::MailPolicy);
    }
    type PolicyRow = (Vec<u8>, i64, Vec<u8>, Vec<u8>, Vec<u8>, i64, i64);
    let stored: Option<PolicyRow> = db
        .query_row(
            "SELECT issuer, issuer_epoch, custodian, scope, recipient,
                max_bytes, max_offers FROM custodian_mailbox_policy
         WHERE mailbox=?1",
            params![p.mailbox.as_slice()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| MailRoleHold::MailPolicy)?;
    let Some((owner, owner_epoch, custodian, scope, recipient, max_bytes, max_offers)) = stored
    else {
        return Err(MailRoleHold::MailPolicy);
    };
    if owner.as_slice() != p.issuer.as_slice()
        || owner_epoch != p.issuer_epoch as i64
        || custodian.as_slice() != p.custodian.as_slice()
        || scope.as_slice() != p.scope.as_slice()
        || recipient.as_slice() != p.recipient.as_slice()
        || max_bytes != i64::from(p.max_message_bytes)
        || max_offers != i64::from(p.max_offers)
    {
        return Err(MailRoleHold::MailPolicy);
    }
    Ok(())
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
        let policy = VerifiedMailRoleLab::open(mail_policy_folder, node, node_endpoint, now_ms)?;
        let claims = policy.claims;
        if claims.custodian_public != custodian_signer.verifying_key().to_bytes() {
            return Err(MailRoleHold::UnpinnedTrust);
        }
        let trust = policy.registry()?;
        let db_info = private_existing_db(state)?;
        let db_path = state.join(DB);
        let journal = Journal::open(&db_path).map_err(|_| MailRoleHold::Journal)?;
        require_exact_mailbox(state, &policy)?;
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
        // Check the exact filesystem object before opening SQLite by path;
        // a replaced/symlinked DB is an unsafe-file error, never a policy row.
        let info = private_existing_db(&self.state)?;
        if (info.dev(), info.ino()) != self.db_inode {
            return Err(MailRoleHold::UnsafeDatabase);
        }
        self.policy.require_current(node, now_ms)?;
        require_exact_mailbox(&self.state, &self.policy)?;
        if self.policy.claims.custodian_public != self.custodian_signer.verifying_key().to_bytes() {
            return Err(MailRoleHold::UnpinnedTrust);
        }
        // The path must still refer to the original inode after the SQL
        // query; the lab gate does not permit path replacement during use.
        let rechecked = private_existing_db(&self.state)?;
        if (rechecked.dev(), rechecked.ino()) != self.db_inode {
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
