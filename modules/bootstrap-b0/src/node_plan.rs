//! B0 Node v2/v3 canonical plans and detached-review-only scope.
use super::*;

pub(super) const NODE_V2_ACTIONS: [ProposedChange; 12] = [
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
// B0 v3 adds the exact separately signed bootstrap-helper digest to the
// reviewed owner scope. The v1/v2 JSON and hash domains stay byte compatible.
pub(super) const NODE_V3_ACTIONS: [ProposedChange; 14] = [
    ProposedChange::VerifySignedPinnedPackage,
    ProposedChange::RequireIndependentlyPinnedRootBootstrap,
    ProposedChange::VerifySignedBootstrapHelperBeforeRootExecution,
    ProposedChange::InstallRootOwnedBootstrapHelperAtFixedPath,
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

impl DryRunPlan {
    fn canonical_bytes(&self) -> Vec<u8> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            schema_version: u16,
            install_id: Uuid,
            target: &'a ServerTarget,
            package: &'a PackagePin,
            #[serde(skip_serializing_if = "Option::is_none")]
            bootstrap_helper_sha256: Option<[u8; 32]>,
            #[serde(skip_serializing_if = "Option::is_none")]
            bootstrap_helper_install_path: Option<&'a str>,
            inspected_host: &'a ReadOnlyHostFacts,
            proposed_changes: &'a [ProposedChange],
        }
        serde_json::to_vec(&Canonical {
            schema_version: self.schema_version,
            install_id: self.install_id,
            target: &self.target,
            package: &self.package,
            bootstrap_helper_sha256: self.bootstrap_helper_sha256,
            bootstrap_helper_install_path: self.bootstrap_helper_install_path.as_deref(),
            inspected_host: &self.inspected_host,
            proposed_changes: &self.proposed_changes,
        }).expect("typed canonical preview must serialize")
    }

    pub fn computed_digest(&self) -> String {
        let domain = match self.schema_version {
            2 => "Akula Node B1-E2E-07 signed owner review plan v2",
            3 => "Akula Node B1-E2E-07 signed bootstrap and owner review plan v3",
            _ => "Akula Bootstrap B0 readonly plan v1",
        };
        let mut hash = blake3::Hasher::new_derive_key(domain);
        hash.update(&self.canonical_bytes());
        hash.finalize().to_hex().to_string()
    }

    pub fn verify_unchanged(&self) -> bool {
        let fixed = match self.schema_version {
            1 => self.proposed_changes == FIXED_ACTIONS
                && self.bootstrap_helper_sha256.is_none()
                && self.bootstrap_helper_install_path.is_none(),
            2 => self.proposed_changes == NODE_V2_ACTIONS
                && self.bootstrap_helper_sha256.is_none()
                && self.bootstrap_helper_install_path.is_none(),
            3 => self.proposed_changes == NODE_V3_ACTIONS
                && self.bootstrap_helper_install_path.as_deref()
                    == Some(ROOT_HELPER_EXECUTABLE_PATH)
                && self.bootstrap_helper_sha256.is_some_and(|helper| {
                    helper != [0; 32] && helper != self.package.package_sha256
                }),
            _ => false,
        };
        fixed && self.computed_digest() == self.plan_blake3
    }
}

pub(super) fn preview_with_scope<T: PreviewTransport>(
    input: &B0Input,
    transport: &mut T,
    cancelled: impl Fn() -> bool,
    node_v2: bool,
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
    // Legacy v1 required an arbitrary sudo listing. Node v2 is a *read-only*
    // inventory and must not claim that sudo listing is install authority.
    // A clean VPS without sudo can still be inventoried as Needs Admin.
    if !node_v2 && !facts.has_sudo_rights { return Err(B0Error::MissingPrivilege); }
    if facts.cpu_count == 0 || facts.ram_mib < MIN_RAM_MIB
        || facts.free_disk_mib < MIN_FREE_DISK_MIB
    {
        return Err(B0Error::InsufficientResources);
    }

    // Returning a plan has NO side effects, even after all checks succeeded.
    // B1 must use a separate, explicitly user-approved transaction type.
    let mut plan = DryRunPlan {
        schema_version: if node_v2 { 2 } else { 1 },
        install_id: input.install_id,
        target: input.target.clone(),
        package: input.package.clone(),
        bootstrap_helper_sha256: None,
        bootstrap_helper_install_path: None,
        inspected_host: facts,
        proposed_changes: if node_v2 {
            NODE_V2_ACTIONS.to_vec()
        } else {
            FIXED_ACTIONS.to_vec()
        },
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
    preview_with_scope(input, transport, cancelled, true)
}

/// A root-staging-capable review MUST show the actual privileged helper's
/// independently authenticated SHA-256 as well as the Node daemon package.
/// It is not derived from SSH output or caller-supplied executable paths.
/// The returned plan is read-only; neither a matching hash nor this preview
/// confers OS root authority or an install/reboot grant.
pub fn preview_node_v3<T: PreviewTransport>(
    input: &B0Input,
    helper_package_sha256: [u8; 32],
    transport: &mut T,
    cancelled: impl Fn() -> bool,
) -> Result<DryRunPlan, B0Error> {
    if helper_package_sha256 == [0; 32]
        || helper_package_sha256 == input.package.package_sha256
    {
        return Err(B0Error::InvalidPackage);
    }
    let mut plan = preview_with_scope(input, transport, cancelled, true)?;
    plan.schema_version = 3;
    plan.bootstrap_helper_sha256 = Some(helper_package_sha256);
    plan.bootstrap_helper_install_path = Some(ROOT_HELPER_EXECUTABLE_PATH.to_owned());
    plan.proposed_changes = NODE_V3_ACTIONS.to_vec();
    plan.plan_blake3 = plan.computed_digest();
    Ok(plan)
}

