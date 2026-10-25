//! AKULA-SERVER-BOOTSTRAP B0: inert installation preview contract.
//!
//! This library CANNOT open SSH sessions, execute commands, send credentials,
//! modify remote hosts, or install Akula. It proves input validation, pinned
//! host-key gating, deterministic preview and cancellation in a mock transport.
//! A later SSH driver must independently authenticate the actual handshake key
//! before it can provide read-only host facts. This module alone is NOT SSH.

use ring::{constant_time, digest};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use uuid::Uuid;

mod node_plan;
use node_plan::{preview_with_scope, NODE_V2_ACTIONS, NODE_V3_ACTIONS};
pub use node_plan::{preview_node_v2, preview_node_v3};
const MIN_RAM_MIB: u64 = 512;
const MIN_FREE_DISK_MIB: u64 = 1024;

/// Fixed root-owned helper path shown to the user in B0 v3 and independently
/// enforced on Linux. This is a PLAN DISCLOSURE, not install permission.
pub const ROOT_HELPER_EXECUTABLE_PATH: &str =
    "/usr/libexec/akula/akula-root-b1d0-helper";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum B0Error {
    InvalidTarget,
    InvalidPackage,
    Cancelled,
    KeyUnavailable,
    HostKeyMismatch,
    InspectionUnavailable,
    UnsupportedOs,
    UnsupportedArchitecture,
    MissingSystemd,
    MissingPrivilege,
    InsufficientResources,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerTarget {
    pub host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    /// Pinned SHA-256 of the actual SSH server public-key *wire encoding*.
    /// Obtain independently from the operator, not from the untrusted SSH peer.
    pub trusted_key_sha256: [u8; 32],
}

impl ServerTarget {
    /// Public for the opt-in Core preflight adapter; never authorizes installation.
    pub fn validate(&self) -> Result<(), B0Error> {
        if self.ssh_port == 0 || self.trusted_key_sha256.iter().all(|byte| *byte == 0) {
            return Err(B0Error::InvalidTarget);
        }
        if self.host.len() > 253 || self.host.is_empty() || !self.host.is_ascii() {
            return Err(B0Error::InvalidTarget);
        }
        if self.host.parse::<IpAddr>().is_err() && !valid_dns_name(&self.host) {
            return Err(B0Error::InvalidTarget);
        }
        // Initial bootstrap profile uses one unprivileged POSIX account.
        // Reject usernames that could become shell/option syntax later.
        let user = self.ssh_user.as_bytes();
        if user.is_empty() || user.len() > 32 || user == b"root"
            || !matches!(user[0], b'a'..=b'z' | b'_')
            || !user.iter().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()
                || *b == b'_' || *b == b'-')
        {
            return Err(B0Error::InvalidTarget);
        }
        Ok(())
    }
}

fn valid_dns_name(host: &str) -> bool {
    host.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63
            && label.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
            && label.as_bytes().last().is_some_and(u8::is_ascii_alphanumeric)
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportedOs {
    Debian,
    Ubuntu,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineArch {
    X86_64,
    Aarch64,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadOnlyHostFacts {
    pub os: SupportedOs,
    pub arch: MachineArch,
    pub has_systemd: bool,
    pub has_sudo_rights: bool,
    pub cpu_count: u16,
    pub ram_mib: u64,
    pub free_disk_mib: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePin {
    /// Version and trusted signing-key label must be supplied from the
    /// separately authenticated release manifest, NOT from the SSH server.
    pub version: String,
    pub package_sha256: [u8; 32],
    pub trusted_signer_id: String,
}

impl PackagePin {
    /// Public for the opt-in Core preflight adapter; never authorizes installation.
    pub fn validate(&self) -> Result<(), B0Error> {
        if self.version.is_empty() || self.version.len() > 64
            || !self.version.bytes().all(|b| b.is_ascii_alphanumeric()
                || b == b'.' || b == b'+' || b == b'-')
            || self.package_sha256.iter().all(|b| *b == 0)
            || self.trusted_signer_id.is_empty()
            || self.trusted_signer_id.len() > 128
            || !self.trusted_signer_id.bytes().all(|b| b.is_ascii_alphanumeric()
                || b == b'.' || b == b'-' || b == b'_')
        {
            return Err(B0Error::InvalidPackage);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct B0Input {
    pub install_id: Uuid,
    pub target: ServerTarget,
    pub package: PackagePin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProposedChange {
    VerifySignedPinnedPackage,
    CreateDedicatedServiceAccount,
    CreateOwnedRuntimeAndStateDirectories,
    WriteRestrictedSystemdUnit,
    EnableLocalLoopbackOnly, // Legacy B0 v1 preview; NOT a Node-V1 install scope.
    VerifyRestartAndSyntheticReadback,
    // B1-E2E-07 V2: explicitly disclose actual service privileges and endpoint.
    RequireIndependentlyPinnedRootBootstrap,
    VerifySignedBootstrapHelperBeforeRootExecution,
    InstallRootOwnedBootstrapHelperAtFixedPath,
    VerifySeparateOwnerSignatureAndDurableCas,
    CommitSignedNodeIntoRootOwnedOpt,
    EnrollPersistentNodeAndReceiptIdentities,
    PinRootOwnedOwnerPolicyV3,
    BindPublicUdp43871WithoutChangingFirewall,
    StartNonRootSystemdService,
    VerifyRebootAndSignedReceipt,
}

const FIXED_ACTIONS: [ProposedChange; 6] = [
    ProposedChange::VerifySignedPinnedPackage,
    ProposedChange::CreateDedicatedServiceAccount,
    ProposedChange::CreateOwnedRuntimeAndStateDirectories,
    ProposedChange::WriteRestrictedSystemdUnit,
    ProposedChange::EnableLocalLoopbackOnly,
    ProposedChange::VerifyRestartAndSyntheticReadback,
];
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DryRunPlan {
    pub schema_version: u16,
    pub install_id: Uuid,
    pub target: ServerTarget,
    pub package: PackagePin,
    /// Exact independently B1a-verified privileged helper package, distinct
    /// from the Node daemon. V1/V2 MUST leave this absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_helper_sha256: Option<[u8; 32]>,
    /// The EXACT future UID0 0500 helper install destination. V1/V2 omit.
    /// V3 has no variable root path and no root execution permission.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bootstrap_helper_install_path: Option<String>,
    pub inspected_host: ReadOnlyHostFacts,
    pub proposed_changes: Vec<ProposedChange>,
    /// Hash of this exact typed plan; NOT a signature and NOT permission to execute.
    pub plan_blake3: String,
}

/// This B0 probe has no method for authentication, arbitrary commands,
/// installation, sudo, file writes, public network changes or key storage.
/// A real SSH implementation is intentionally absent. The next B0 stage must
/// bind the *observed* host key to the actual SSH handshake, and only then
/// authenticate to obtain host facts using a fixed allowlist of read probes.
pub trait PreviewTransport {
    /// Expose actual SSH handshake public-key wire bytes before credentials.
    fn handshake_public_key(&mut self, target: &ServerTarget)
        -> Result<Vec<u8>, B0Error>;

    /// Read-only inspection, called only after the pinned fingerprint matches.
    fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error>;
}

/// Historical B0 v1 remains a read-only inventory with its original sudo
/// listing precondition. The v1 plan is never valid for public Node install.
pub fn preview<T: PreviewTransport>(
    input: &B0Input,
    transport: &mut T,
    cancelled: impl Fn() -> bool,
) -> Result<DryRunPlan, B0Error> {
    preview_with_scope(input, transport, cancelled, false)
}

#[cfg(test)]
mod node_plan_v2_tests;
