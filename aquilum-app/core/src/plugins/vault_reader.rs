//! PLUGIN-04: read-only host facade for registered vault notes.
//!
//! No code here is a WASM/JS import. In particular, bind_host_note must only
//! receive paths from trusted host workspace/document indexing, never raw guest
//! requests. OS directory/file handles stay within trusted Core code.

use super::{
    BrokerError, Capability, HostContext, NoteId, NoteResource, PermissionBroker,
    PluginInstanceHandle, RelativeVaultPath, Resource, VaultId,
};
use crate::app_core::Core;
use crate::sync::{LocalFileSnapshot, PinnedFileIdentity, RegisteredReadAdmission, SyncRelativePath, VaultPortError};
use std::path::Path;

/// Opaque host-side note handle; future guest RPC uses only a mapped token.
/// This type has no serde implementation, public fields or public constructor.
#[derive(Clone, Debug)]
pub struct ScopedNoteHandle {
    vault: VaultId,
    id: NoteId,
    path: RelativeVaultPath,
    relative: SyncRelativePath,
    file_identity: PinnedFileIdentity,
}

impl ScopedNoteHandle {
    /// Trusted host only, not a guest-serializable identifier.
    pub fn host_note_id(&self) -> NoteId {
        self.id
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum PluginReadError {
    Denied,
    InvalidHostPath,
    Unavailable,
}

impl From<BrokerError> for PluginReadError {
    fn from(_: BrokerError) -> Self {
        Self::Denied
    }
}
impl From<VaultPortError> for PluginReadError {
    fn from(_: VaultPortError) -> Self {
        Self::Unavailable
    }
}

/// Host-only reader whose vault identity derives from Core UiState registration.
pub struct HostPluginVaultReader<'a> {
    core: &'a Core,
    broker: &'a PermissionBroker,
    admission: RegisteredReadAdmission<'a>,
    vault: VaultId,
}

impl<'a> HostPluginVaultReader<'a> {
    pub fn open_registered(
        core: &'a Core,
        broker: &'a PermissionBroker,
        selected_root: &Path,
    ) -> Result<Self, PluginReadError> {
        let admission = RegisteredReadAdmission::open(core, selected_root)?;
        let vault = VaultId::from_uuid_for_host(admission.registered_workspace_id());
        Ok(Self { core, broker, admission, vault })
    }

    /// For trusted host installation/consent; never take UUIDs from a plugin.
    pub fn host_vault_id(&self) -> VaultId {
        self.vault
    }

    /// Binds a host-indexed path to one registered vault and UI document ID.
    /// No file bytes are read and no grant is created. The trusted host, not
    /// the guest, must select the path.
    pub fn bind_host_note(&self, indexed_path: &str) -> Result<ScopedNoteHandle, PluginReadError> {
        self.admission.recheck_registered_root()?;
        let relative = SyncRelativePath::parse(indexed_path)
            .map_err(|_| PluginReadError::InvalidHostPath)?;
        if relative.as_str() != indexed_path {
            return Err(PluginReadError::InvalidHostPath);
        }
        let path = RelativeVaultPath::parse_host_path(indexed_path)
            .map_err(|_| PluginReadError::InvalidHostPath)?;
        let file_identity = self.admission.bind_plugin_file_identity(&relative)?;
        let id = self.core.ui_state
            .resolve_document(self.admission.registered_workspace_id(), indexed_path)
            .map_err(|_| PluginReadError::Unavailable)?;
        Ok(ScopedNoteHandle {
            vault: self.vault,
            id: NoteId::from_uuid_for_host(id),
            path,
            relative,
            file_identity,
        })
    }

    /// Reads a previously host-bound note; max_bytes is bounded by the pinned
    /// reader. Live context must come from trusted UI state and is queried
    /// again after I/O. A revoked grant cannot release bytes after revalidation.
    pub fn read_note_bounded(
        &self,
        instance: PluginInstanceHandle,
        note: &ScopedNoteHandle,
        max_bytes: u64,
        live_context: impl Fn() -> HostContext,
    ) -> Result<LocalFileSnapshot, PluginReadError> {
        self.read_with_hook(instance, note, max_bytes, live_context, || {})
    }

    /// Host-only bridge from a physically pinned local note to a verified
    /// CURRENT Sync revision. The caller supplies a trusted, already-loaded
    /// RevisionDag; the note path always comes from a host-created handle,
    /// never from plugin JSON or user-controlled search parameters.
    ///
    /// This deliberately requests NotesRead (not just NotesSearch) because
    /// binding needs to inspect protected primary bytes. Existing read
    /// admission refuses substituted inodes, unsafe roots and open editors.
    /// We re-read and revalidate permission after resolving the DAG binding.
    ///
    /// A matching snapshot proves neither the future state of the file nor
    /// authority to release search results over RPC. The real search/response
    /// boundary must revalidate source and grants again. No historical access
    /// or multi-head auto-resolution is provided by this bridge.
    pub(crate) fn bind_current_sync_note(
        &self,
        instance: PluginInstanceHandle,
        note: &ScopedNoteHandle,
        dag: &crate::sync::RevisionDag,
        max_bytes: u64,
        live_context: impl Fn() -> HostContext,
    ) -> Result<crate::search::semantic_probe::OwnedSelection, PluginReadError> {
        let first = self.read_note_bounded(
            instance, note, max_bytes, &live_context,
        )?;
        let selected = crate::search::semantic_probe::bind_current_note_to_sync(
            dag, note.relative.as_str(), &first.content_hash,
        ).map_err(|_| PluginReadError::Unavailable)?;
        let second = self.read_note_bounded(
            instance, note, max_bytes, &live_context,
        )?;
        if second.content_hash != first.content_hash || second.bytes.len() != first.bytes.len() {
            return Err(PluginReadError::Unavailable);
        }
        Ok(selected)
    }

    pub(super) fn read_with_hook(
        &self,
        instance: PluginInstanceHandle,
        note: &ScopedNoteHandle,
        max_bytes: u64,
        live_context: impl Fn() -> HostContext,
        after_read: impl FnOnce(),
    ) -> Result<LocalFileSnapshot, PluginReadError> {
        if note.vault != self.vault {
            return Err(PluginReadError::Denied);
        }
        let context = live_context();
        if context.vault != self.vault {
            return Err(PluginReadError::Denied);
        }
        let resource = Resource::Note(NoteResource {
            vault: self.vault,
            id: note.id,
            path: note.path.clone(),
        });
        // Authorize before looking up or opening the file's contents.
        let lease = self.broker.authorize(
            instance, context, Capability::NotesRead, &resource,
        )?;
        let bytes = self.admission.read_plugin_identified_bounded(&note.relative, note.file_identity, max_bytes)?;
        after_read();
        self.admission.recheck_registered_root()?;
        self.broker.revalidate_lease(&lease, live_context())?;
        Ok(bytes)
    }
}
