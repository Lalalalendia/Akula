//! PR 00: inert cloud-sync contracts, types and deterministic fake provider.
//!
//! The existing local note history is not the distributed revision graph.
//! Later stages must use the File Gate for writes and protect dirty editor buffers.

mod core_scan;
mod dag;
mod download;
mod dag_store;
pub mod fake;
mod fake_vault;
mod host_admission;
pub(crate) use host_admission::RegisteredReadAdmission;
pub(crate) use pinned_read::PinnedFileIdentity;
#[cfg(test)]
mod fault_matrix;
mod identity;
mod journal;
mod local_create;
mod local_recovery;
mod planner;
mod protection_store;
mod path;
mod pinned_read;
#[cfg(windows)]
mod pinned_windows;
mod protocol;
mod provider;
mod upload;
mod vault;

pub use core_scan::{CoreVaultScanner, LocalScanPreview, LocalScanTrust, ScanLimits};
pub use dag::{DagError, IngestOutcome, RevisionDag, RevisionOperation, RevisionRecord};
pub use dag_store::{DagStoreError, DagStoreResult, SqliteDagStore};
pub use download::{stage_remote_download, StageDownloadError, StagedDownload};
pub use identity::{DeviceId, DocumentId, OperationId, RevisionId, VaultId};
pub use journal::{
    ConfirmedBaseline, JournalAction, JournalError, JournalPhase, JournalRecord, JournalResult,
    OperationIntent, LocalPublicationReview, RecoveryAction, RecoveryItem, RemoteGuard, SqliteSyncJournal,
};
pub use planner::{
    plan_snapshot, HoldReason, ObservedObject, PlannedAction, SafetyPolicy, SnapshotDiff,
};
pub use path::{reject_path_collisions, resolve_no_symlinks, SyncPathError, SyncRelativePath};
pub use protocol::{
    decode_identity_for_write, IdentityEnvelope, IdentityRecord, ProtocolCompatibility,
    ProtocolError, ProtocolVersion, SyncConfig, SyncProvider, WireEnvelope,
};

pub use fake::FakeRemoteStore;
pub use fake_vault::FakeVaultPort;
pub use provider::{
    collect_verified_listing, ProviderCapabilities, RemoteObject, RemotePage, RemotePrecondition,
    RemoteSnapshot, RemoteStore, RemoteStoreError, RemoteWriteReceipt,
};
pub use upload::{upload_immutable_verified, UploadOutcome, VerifiedUploadError};
pub use vault::{
    LocalFileEntry, LocalFileSnapshot, PreservationReceipt, VaultFileKind, VaultPort,
    VaultPortError,
};

/// No sync scheduler or external I/O is started by importing these contracts.
/// Runtime activation/UI remain out of scope until the S3 slice is validated.
pub const SYNC_ENGINE_FEATURE_ENABLED: bool = cfg!(feature = "sync-engine");

#[cfg(all(test, not(feature = "sync-engine")))]
mod disabled_tests {
    #[test]
    fn compiled_contracts_do_not_enable_cloud_sync_by_default() {
        assert!(!super::SYNC_ENGINE_FEATURE_ENABLED);
    }
}
