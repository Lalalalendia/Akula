//! PLUGIN-02: in-memory, default-deny capability checks for trusted host calls.
//!
//! This is NOT a guest API, a secure filesystem resolver, a network sandbox,
//! a consent UI, a trusted package verifier, or a durable grant store.
//! Only trusted host code may call register_host_package/grant_host_approved;
//! guest/WIT messages must never be mapped directly to these methods.

use super::{manifest::valid_domain, Capability, PluginManifest};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VaultId(Uuid);

impl VaultId {
    /// Create a host-owned vault identity; never derive rights from a guest string.
    pub fn new_for_host() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NoteId(Uuid);

impl NoteId {
    /// The host must bind this ID to its actual vault document registry.
    pub fn new_for_host() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Unforgeable in a guest protocol: no serde and no public constructor.
/// This is only a host-side in-memory handle, not an OS capability.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PluginInstanceHandle(Uuid);

/// Host-bound package identity: the publisher claim is validated by a future
/// trusted installer, never by accepting guest-provided JSON as authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginIdentity {
    plugin_id: String,
    publisher: String,
    artifact_digest: [u8; 32],
}

impl PluginIdentity {
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    pub fn artifact_digest(&self) -> [u8; 32] {
        self.artifact_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct RelativeVaultPath(String);

impl RelativeVaultPath {
    /// This is a lexical logical-path check, not a filesystem confinement API.
    /// PLUGIN-04 must pin real OS resource identity and reject symlink/junction races.
    pub fn parse_host_path(path: &str) -> Result<Self, BrokerError> {
        if path.is_empty()
            || path.len() > 1024
            || path.starts_with('/')
            || path.contains('\\')
            || path.contains(':')
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(BrokerError::InvalidResource);
        }
        Ok(Self(path.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_inside(&self, folder: &Self) -> bool {
        self == folder
            || self
                .0
                .strip_prefix(folder.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    }
}

/// Exact, lower-case DNS host only; not a replacement for an SSRF-safe network
/// broker that revalidates resolution, redirects, proxies and actual sockets.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct NetworkHost(String);

impl NetworkHost {
    pub fn parse_declared_host(raw: &str) -> Result<Self, BrokerError> {
        if !valid_domain(raw) {
            return Err(BrokerError::InvalidResource);
        }
        Ok(Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Host-resolved document identity. The path has NOT been opened or pinned.
/// Do not construct these from guest-supplied IDs/paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteResource {
    pub vault: VaultId,
    pub id: NoteId,
    pub path: RelativeVaultPath,
}

impl NoteResource {
    pub fn from_host_index(
        vault: VaultId,
        id: NoteId,
        logical_path: &str,
    ) -> Result<Self, BrokerError> {
        Ok(Self {
            vault,
            id,
            path: RelativeVaultPath::parse_host_path(logical_path)?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resource {
    /// Register a native command or panel. No vault note data is carried.
    Ui,
    /// Host-resolved note or attachment, also used for individual search hits.
    Note(NoteResource),
    /// Declared network destination. This grants no actual network socket.
    Network(NetworkHost),
    /// Clipboard/AI action; note data must be authorized separately.
    HostAction,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrantScope {
    /// UI / clipboard / AI host action: no implicit access to note contents.
    Action,
    CurrentNote,
    SelectedNotes(BTreeSet<NoteId>),
    Folder(RelativeVaultPath),
    WholeVault,
    NetworkHost(NetworkHost),
}

impl GrantScope {
    pub fn selected_notes(notes: impl IntoIterator<Item = NoteId>) -> Result<Self, BrokerError> {
        let selected: BTreeSet<_> = notes.into_iter().collect();
        if selected.is_empty() {
            return Err(BrokerError::InvalidScope);
        }
        Ok(Self::SelectedNotes(selected))
    }

    pub fn folder(path: &str) -> Result<Self, BrokerError> {
        Ok(Self::Folder(RelativeVaultPath::parse_host_path(path)?))
    }

    pub fn network_host(name: &str) -> Result<Self, BrokerError> {
        Ok(Self::NetworkHost(NetworkHost::parse_declared_host(name)?))
    }
}

/// Host-controlled active selection. Guest requests must never set this context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostContext {
    pub vault: VaultId,
    pub current_note: Option<NoteId>,
}

impl HostContext {
    pub fn from_host_state(vault: VaultId, current_note: Option<NoteId>) -> Self {
        Self {
            vault,
            current_note,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerError {
    InvalidManifest,
    InvalidArtifact,
    InvalidResource,
    InvalidScope,
    UnknownInstance,
    WrongVault,
    UndeclaredCapability,
    Denied,
    StaleLease,
    RevisionExhausted,
    StatePoisoned,
}

impl fmt::Display for BrokerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "plugin permission broker rejected request: {self:?}")
    }
}

impl std::error::Error for BrokerError {}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Grant {
    capability: Capability,
    scope: GrantScope,
}

struct RegisteredInstance {
    vault: VaultId,
    /// The host, never the guest, supplies the installed package identity.
    identity: PluginIdentity,
    /// Only capabilities declared by the pinned manifest may be consented to.
    declarations: BTreeSet<u8>,
    network_domains: Vec<NetworkHost>,
    grants: Vec<Grant>,
    revision: u64,
}

#[derive(Default)]
struct BrokerState {
    instances: HashMap<PluginInstanceHandle, RegisteredInstance>,
}

#[derive(Default)]
pub struct PermissionBroker {
    state: Mutex<BrokerState>,
}

/// An authorization snapshot valid only while instance/rights/context are unchanged.
/// Do NOT treat this as a durable write reservation: PLUGIN-09 must recheck
/// under the document transaction before committing any side effect.
#[derive(Clone, Debug)]
pub struct AuthorizationLease {
    instance: PluginInstanceHandle,
    context: HostContext,
    capability: Capability,
    resource: Resource,
    revision: u64,
    artifact_digest: [u8; 32],
}

impl PermissionBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// TRUSTED HOST ONLY. Caller must have validated package origin and supplied
    /// a digest of installed bytes. Verification/installation is NOT performed here.
    /// The supplied manifest is still parsed data, never a grant.
    pub fn register_host_package(
        &self,
        manifest: &PluginManifest,
        artifact_digest: [u8; 32],
        vault: VaultId,
    ) -> Result<PluginInstanceHandle, BrokerError> {
        manifest.validate().map_err(|_| BrokerError::InvalidManifest)?;
        if artifact_digest == [0; 32] {
            return Err(BrokerError::InvalidArtifact);
        }
        let mut state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        let handle = loop {
            let candidate = PluginInstanceHandle(Uuid::new_v4());
            if !state.instances.contains_key(&candidate) {
                break candidate;
            }
        };
        let declarations = manifest
            .permissions
            .required
            .iter()
            .chain(manifest.permissions.optional.iter())
            .map(|c| capability_tag(*c))
            .collect();
        let network_domains = manifest
            .network_domains
            .iter()
            .map(|s| NetworkHost::parse_declared_host(s))
            .collect::<Result<Vec<_>, _>>()?;
        state.instances.insert(
            handle,
            RegisteredInstance {
                vault,
                identity: PluginIdentity {
                    plugin_id: manifest.id.clone(),
                    publisher: manifest.publisher.clone(),
                    artifact_digest,
                },
                declarations,
                network_domains,
                grants: Vec::new(),
                revision: 0,
            },
        );
        Ok(handle)
    }

    /// Host-side inventory for future consent/audit UI, not a guest API.
    pub fn host_instance_identity(
        &self,
        handle: PluginInstanceHandle,
    ) -> Result<PluginIdentity, BrokerError> {
        let state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        let record = state.instances.get(&handle).ok_or(BrokerError::UnknownInstance)?;
        Ok(record.identity.clone())
    }

    /// TRUSTED HOST CONSENT PATH ONLY. This does not run a consent UI (PLUGIN-11).
    /// A manifest's requests never invoke this method automatically.
    pub fn grant_host_approved(
        &self,
        handle: PluginInstanceHandle,
        capability: Capability,
        scope: GrantScope,
    ) -> Result<(), BrokerError> {
        if !scope_is_valid(capability, &scope) {
            return Err(BrokerError::InvalidScope);
        }
        let mut state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        let record = state.instances.get_mut(&handle).ok_or(BrokerError::UnknownInstance)?;
        if !record.declarations.contains(&capability_tag(capability)) {
            return Err(BrokerError::UndeclaredCapability);
        }
        if let GrantScope::NetworkHost(domain) = &scope {
            if !record.network_domains.contains(domain) {
                return Err(BrokerError::UndeclaredCapability);
            }
        }
        let grant = Grant { capability, scope };
        if !record.grants.contains(&grant) {
            let next_revision = record
                .revision
                .checked_add(1)
                .ok_or(BrokerError::RevisionExhausted)?;
            record.grants.push(grant);
            record.revision = next_revision;
        }
        Ok(())
    }

    pub fn authorize(
        &self,
        handle: PluginInstanceHandle,
        context: HostContext,
        capability: Capability,
        resource: &Resource,
    ) -> Result<AuthorizationLease, BrokerError> {
        let state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        let record = state.instances.get(&handle).ok_or(BrokerError::UnknownInstance)?;
        check_request(record, context, capability, resource)?;
        Ok(AuthorizationLease {
            instance: handle,
            context,
            capability,
            resource: resource.clone(),
            revision: record.revision,
            artifact_digest: record.identity.artifact_digest,
        })
    }

    /// Revalidate after asynchronous work and immediately before a side effect.
    /// The downstream document/network operation needs its own atomicity guard.
    pub fn revalidate_lease(
        &self,
        lease: &AuthorizationLease,
        current_context: HostContext,
    ) -> Result<(), BrokerError> {
        let state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        let record = state
            .instances
            .get(&lease.instance)
            .ok_or(BrokerError::UnknownInstance)?;
        if lease.context != current_context
            || lease.revision != record.revision
            || lease.artifact_digest != record.identity.artifact_digest
        {
            return Err(BrokerError::StaleLease);
        }
        check_request(record, current_context, lease.capability, &lease.resource)
    }

    /// In-memory revocation in this stage. PLUGIN-03 will persist, audit and
    /// propagate it to runtime jobs; old leases immediately become stale.
    pub fn revoke_all_for_host(
        &self,
        handle: PluginInstanceHandle,
    ) -> Result<(), BrokerError> {
        let mut state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        let record = state.instances.get_mut(&handle).ok_or(BrokerError::UnknownInstance)?;
        let next_revision = record
            .revision
            .checked_add(1)
            .ok_or(BrokerError::RevisionExhausted)?;
        record.grants.clear();
        record.revision = next_revision;
        Ok(())
    }

    pub fn unregister_host_instance(
        &self,
        handle: PluginInstanceHandle,
    ) -> Result<(), BrokerError> {
        let mut state = self.state.lock().map_err(|_| BrokerError::StatePoisoned)?;
        if state.instances.remove(&handle).is_none() {
            return Err(BrokerError::UnknownInstance);
        }
        Ok(())
    }
}

fn check_request(
    record: &RegisteredInstance,
    context: HostContext,
    capability: Capability,
    resource: &Resource,
) -> Result<(), BrokerError> {
    if record.vault != context.vault {
        return Err(BrokerError::WrongVault);
    }
    if !resource_matches(capability, resource, context) {
        return Err(BrokerError::InvalidResource);
    }
    if !record.declarations.contains(&capability_tag(capability)) {
        return Err(BrokerError::UndeclaredCapability);
    }
    if !record
        .grants
        .iter()
        .any(|grant| grant.capability == capability && grant_allows(&grant.scope, resource, context))
    {
        return Err(BrokerError::Denied);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActionClass {
    Ui,
    VaultNote,
    Network,
    HostAction,
}

fn class(capability: Capability) -> ActionClass {
    match capability {
        Capability::UiCommand | Capability::UiPanel => ActionClass::Ui,
        Capability::NotesRead
        | Capability::NotesSearch
        | Capability::NotesPatch
        | Capability::NotesCreate
        | Capability::NotesTrash
        | Capability::AttachmentsRead
        | Capability::AttachmentsWrite
        | Capability::EventsSubscribe => ActionClass::VaultNote,
        Capability::NetworkFetch => ActionClass::Network,
        Capability::ClipboardRead
        | Capability::ClipboardWrite
        | Capability::AiInvoke => ActionClass::HostAction,
    }
}

fn scope_is_valid(capability: Capability, scope: &GrantScope) -> bool {
    match (class(capability), scope) {
        (ActionClass::Ui | ActionClass::HostAction, GrantScope::Action) => true,
        (ActionClass::Network, GrantScope::NetworkHost(_)) => true,
        (ActionClass::VaultNote, GrantScope::CurrentNote | GrantScope::WholeVault) => true,
        (ActionClass::VaultNote, GrantScope::SelectedNotes(selected)) => !selected.is_empty(),
        (ActionClass::VaultNote, GrantScope::Folder(_)) => true,
        _ => false,
    }
}

fn resource_matches(capability: Capability, resource: &Resource, context: HostContext) -> bool {
    match (class(capability), resource) {
        (ActionClass::Ui, Resource::Ui) => true,
        (ActionClass::HostAction, Resource::HostAction) => true,
        (ActionClass::Network, Resource::Network(_)) => true,
        (ActionClass::VaultNote, Resource::Note(note)) => note.vault == context.vault,
        _ => false,
    }
}

fn grant_allows(scope: &GrantScope, resource: &Resource, context: HostContext) -> bool {
    match (scope, resource) {
        (GrantScope::Action, Resource::Ui | Resource::HostAction) => true,
        (GrantScope::CurrentNote, Resource::Note(note)) => context.current_note == Some(note.id),
        (GrantScope::SelectedNotes(selected), Resource::Note(note)) => selected.contains(&note.id),
        (GrantScope::Folder(folder), Resource::Note(note)) => note.path.is_inside(folder),
        (GrantScope::WholeVault, Resource::Note(_)) => true,
        (GrantScope::NetworkHost(granted), Resource::Network(target)) => granted == target,
        _ => false,
    }
}

// Keep this mapping explicit. A future permission added to PluginManifest
// must deliberately receive a tag and a resource classification.
// These are private policy keys, not a stable wire representation.
fn capability_tag(capability: Capability) -> u8 {
    match capability {
        Capability::UiCommand => 1,
        Capability::UiPanel => 2,
        Capability::NotesRead => 3,
        Capability::NotesSearch => 4,
        Capability::NotesPatch => 5,
        Capability::NotesCreate => 6,
        Capability::NotesTrash => 7,
        Capability::AttachmentsRead => 8,
        Capability::AttachmentsWrite => 9,
        Capability::EventsSubscribe => 10,
        Capability::NetworkFetch => 11,
        Capability::ClipboardRead => 12,
        Capability::ClipboardWrite => 13,
        Capability::AiInvoke => 14,
    }
}
