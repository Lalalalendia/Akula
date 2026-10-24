//! AKULA-SERVER-BOOTSTRAP B0: inert installation preview contract.
//!
//! This library CANNOT open SSH sessions, execute commands, send credentials,
//! modify remote hosts, or install Akula. It proves input validation, pinned
//! host-key gating, deterministic preview and cancellation in a mock transport.
//! A later SSH driver must independently authenticate the actual handshake key
//! before it can provide read-only host facts. This module alone is NOT SSH.

use ring::{constant_time, digest};
use serde::Serialize;
use std::net::IpAddr;
use uuid::Uuid;

const MIN_RAM_MIB: u64 = 512;
const MIN_FREE_DISK_MIB: u64 = 1024;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SupportedOs {
    Debian,
    Ubuntu,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MachineArch {
    X86_64,
    Aarch64,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReadOnlyHostFacts {
    pub os: SupportedOs,
    pub arch: MachineArch,
    pub has_systemd: bool,
    pub has_sudo_rights: bool,
    pub cpu_count: u16,
    pub ram_mib: u64,
    pub free_disk_mib: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProposedChange {
    VerifySignedPinnedPackage,
    CreateDedicatedServiceAccount,
    CreateOwnedRuntimeAndStateDirectories,
    WriteRestrictedSystemdUnit,
    EnableLocalLoopbackOnly, // Legacy B0 v1 preview; NOT a Node-V1 install scope.
    VerifyRestartAndSyntheticReadback,
    // B1-E2E-07 V2: explicitly disclose actual service privileges and endpoint.
    RequireIndependentlyPinnedRootBootstrap,
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
const NODE_V2_ACTIONS: [ProposedChange; 12] = [
    ProposedChange::VerifySignedPinnedPackage,
    ProposedChange::RequireIndependentlyPinnedRootBootstrap,
    ProposedChange::VerifySeparateOwnerSignatureAndDurableCas,
    ProposedChange::CommitSignedNodeIntoRootOwnedOpt,
    ProposedChange::CreateDedicatedServiceAccount,
    ProposedChange::CreateOwnedRuntimeAndStateDirectories,
    ProposedChange::EnrollPersistentNodeAndReceiptIdentities,
    ProposedChange::PinRootOwnedOwnerPolicyV3,
    ProposedChange::WriteRestrictedSystemdUnit,
    ProposedChange::BindPublicUdp43871WithoutChangingFirewall,
    ProposedChange::StartNonRootSystemdService,
    ProposedChange::VerifyRebootAndSignedReceipt,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DryRunPlan {
    pub schema_version: u16,
    pub install_id: Uuid,
    pub target: ServerTarget,
    pub package: PackagePin,
    pub inspected_host: ReadOnlyHostFacts,
    pub proposed_changes: Vec<ProposedChange>,
    /// Hash of this exact typed plan; NOT a signature and NOT permission to execute.
    pub plan_blake3: String,
}

impl DryRunPlan {
    fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            schema_version: u16,
            install_id: Uuid,
            target: &'a ServerTarget,
            package: &'a PackagePin,
            inspected_host: &'a ReadOnlyHostFacts,
            proposed_changes: &'a [ProposedChange],
        }
        serde_json::to_vec(&Canonical {
            schema_version: self.schema_version,
            install_id: self.install_id,
            target: &self.target,
            package: &self.package,
            inspected_host: &self.inspected_host,
            proposed_changes: &self.proposed_changes,
        }).expect("typed canonical preview must serialize")
    }

    pub fn computed_digest(&self) -> String {
        let domain = match self.schema_version {
            2 => "Akula Node B1-E2E-07 signed owner review plan v2",
            _ => "Akula Bootstrap B0 readonly plan v1",
        };
        let mut hash = blake3::Hasher::new_derive_key(domain);
        hash.update(&self.canonical_bytes());
        hash.finalize().to_hex().to_string()
    }

    pub fn verify_unchanged(&self) -> bool {
        let fixed = match self.schema_version {
            1 => self.proposed_changes == FIXED_ACTIONS,
            2 => self.proposed_changes == NODE_V2_ACTIONS,
            _ => false,
        };
        fixed && self.computed_digest() == self.plan_blake3
    }
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

pub fn preview<T: PreviewTransport>(
    input: &B0Input,
    transport: &mut T,
    cancelled: impl Fn() -> bool,
) -> Result<DryRunPlan, B0Error> {
    input.target.validate()?;
    input.package.validate()?;
    if input.install_id.is_nil() { return Err(B0Error::InvalidTarget); }
    if cancelled() { return Err(B0Error::Cancelled); }

    let key = transport.handshake_public_key(&input.target)?;
    // A huge peer-supplied blob must not be retained/hashed without bounds.
    if key.len() < 16 || key.len() > 16 * 1024 {
        return Err(B0Error::KeyUnavailable);
    }
    let observed = digest::digest(&digest::SHA256, &key);
    if constant_time::verify_slices_are_equal(
        observed.as_ref(), &input.target.trusted_key_sha256
    ).is_err() {
        return Err(B0Error::HostKeyMismatch);
    }
    if cancelled() { return Err(B0Error::Cancelled); }

    let facts = transport.inspect_host()?;
    if cancelled() { return Err(B0Error::Cancelled); }

    match facts.os {
        SupportedOs::Debian | SupportedOs::Ubuntu => {}
        SupportedOs::Other => return Err(B0Error::UnsupportedOs),
    }
    if facts.arch != MachineArch::X86_64 {
        return Err(B0Error::UnsupportedArchitecture);
    }
    if !facts.has_systemd { return Err(B0Error::MissingSystemd); }
    if !facts.has_sudo_rights { return Err(B0Error::MissingPrivilege); }
    if facts.cpu_count == 0 || facts.ram_mib < MIN_RAM_MIB
        || facts.free_disk_mib < MIN_FREE_DISK_MIB
    {
        return Err(B0Error::InsufficientResources);
    }

    // Returning a plan has NO side effects, even after all checks succeeded.
    // B1 must use a separate, explicitly user-approved transaction type.
    let mut plan = DryRunPlan {
        schema_version: 1,
        install_id: input.install_id,
        target: input.target.clone(),
        package: input.package.clone(),
        inspected_host: facts,
        proposed_changes: FIXED_ACTIONS.to_vec(),
        plan_blake3: String::new(),
    };
    plan.plan_blake3 = plan.computed_digest();
    Ok(plan)
}

/// Node-V1's actual public listener is UDP 43871, NOT loopback-only.
/// This separate read-only v2 preview retains the verified SSH host facts
/// but binds the correct privileged action set under a distinct hash domain.
/// Historical B0 v1 preview/digests remain compatible with older consumers.
///
/// No user approval, root helper, port listener, firewall write, or install
/// occurs here; the installed Akula MUST not enable InstallServer from this.
pub fn preview_node_v2<T: PreviewTransport>(
    input: &B0Input,
    transport: &mut T,
    cancelled: impl Fn() -> bool,
) -> Result<DryRunPlan, B0Error> {
    let mut plan = preview(input, transport, &cancelled)?;
    if cancelled() { return Err(B0Error::Cancelled); }
    plan.schema_version = 2;
    plan.proposed_changes = NODE_V2_ACTIONS.to_vec();
    plan.plan_blake3 = plan.computed_digest();
    Ok(plan)
}

#[cfg(test)]
mod node_plan_v2_tests {
    use super::*;

    struct StaticKex {
        wire: Vec<u8>,
    }
    impl PreviewTransport for StaticKex {
        fn handshake_public_key(&mut self, _: &ServerTarget) -> Result<Vec<u8>, B0Error> {
            Ok(self.wire.clone())
        }
        fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
            Ok(ReadOnlyHostFacts {
                os: SupportedOs::Ubuntu,
                arch: MachineArch::X86_64,
                has_systemd: true,
                has_sudo_rights: true,
                cpu_count: 4,
                ram_mib: 4096,
                free_disk_mib: 8192,
            })
        }
    }

    #[test]
    fn node_v2_discloses_public_udp_and_separates_legacy_plan_digest() {
        let wire = vec![0x91; 32];
        let input = B0Input {
            install_id: Uuid::from_bytes([1; 16]),
            target: ServerTarget {
                host: "vps.example.org".into(),
                ssh_port: 22,
                ssh_user: "ubuntu".into(),
                trusted_key_sha256: digest::digest(&digest::SHA256, &wire)
                    .as_ref().try_into().unwrap(),
            },
            package: PackagePin {
                version: "0.3.13".into(),
                package_sha256: [0x44; 32],
                trusted_signer_id: "test-release-anchor".into(),
            },
        };
        let old = preview(&input, &mut StaticKex { wire: wire.clone() }, || false).unwrap();
        let v2 = preview_node_v2(&input, &mut StaticKex { wire }, || false).unwrap();
        assert!(old.verify_unchanged() && v2.verify_unchanged());
        assert_eq!((old.schema_version, v2.schema_version), (1, 2));
        assert_ne!(old.plan_blake3, v2.plan_blake3);
        assert_eq!(v2.proposed_changes.as_slice(), NODE_V2_ACTIONS.as_slice());
        assert!(v2.proposed_changes.contains(
            &ProposedChange::BindPublicUdp43871WithoutChangingFirewall
        ));
        assert!(!v2.proposed_changes.contains(&ProposedChange::EnableLocalLoopbackOnly));
        assert!(old.proposed_changes.contains(&ProposedChange::EnableLocalLoopbackOnly));
        for index in 0..v2.proposed_changes.len() {
            let mut changed = v2.clone();
            changed.proposed_changes.remove(index);
            changed.plan_blake3 = changed.computed_digest();
            assert!(!changed.verify_unchanged(), "missing action {index}");
        }
        let mut downgrade = v2.clone();
        downgrade.schema_version = 1;
        downgrade.plan_blake3 = downgrade.computed_digest();
        assert!(!downgrade.verify_unchanged());
        let mut wrong_key = v2;
        wrong_key.target.trusted_key_sha256[0] ^= 1;
        wrong_key.plan_blake3 = wrong_key.computed_digest();
        // A changed target can be internally consistent but is no longer the
        // originally reviewed plan. Detached owner signatures bind its digest.
        assert_ne!(wrong_key.plan_blake3, old.plan_blake3);
    }
}
