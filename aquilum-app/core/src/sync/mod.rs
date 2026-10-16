//! Cloud-sync protocol foundations. No network, filesystem writes or UI activation in PR 01.
//!
//! The existing local note history is not the distributed revision graph.
//! Later stages must use the File Gate for writes and protect dirty editor buffers.

mod identity;
mod path;
mod protocol;

pub use identity::{DeviceId, DocumentId, OperationId, RevisionId, VaultId};
pub use path::{reject_path_collisions, resolve_no_symlinks, SyncPathError, SyncRelativePath};
pub use protocol::{
    decode_identity_for_write, IdentityEnvelope, IdentityRecord, ProtocolCompatibility,
    ProtocolError, ProtocolVersion, SyncConfig, SyncProvider, WireEnvelope,
};
