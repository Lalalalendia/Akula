//! PR 00: inert cloud-sync contracts, types and deterministic fake provider.
//!
//! The existing local note history is not the distributed revision graph.
//! Later stages must use the File Gate for writes and protect dirty editor buffers.

mod core_scan;
mod dag;
mod delta;
mod download;
mod dag_store;
pub mod fake;
mod fake_vault;
mod host_admission;
mod filegate_lock_custody;
mod filegate_custody_identity;
// Installer-provisioned local custody epoch: independent of S3 transport.
// Legacy unhosted Core::open remains a release blocker until native binding.
#[cfg(feature = "host-custody")]
mod host_ledger_identity;
#[cfg(feature = "host-custody")]
pub(crate) mod host_custody_registry;
#[cfg(feature = "sync-installed-host-witness")]
pub(crate) mod host_install_witness;
#[cfg(feature = "sync-installed-host-unix-root-witness")]
mod host_install_unix_root;
#[cfg(all(test, feature = "host-custody"))]
mod host_core_epoch_tests;
pub(crate) mod guarded_note_write;
#[cfg(feature = "sync-s3")]
pub(crate) mod guarded_editor;
#[cfg(test)]
mod cooperative_gate_tests;
pub(crate) mod filegate_lock_probe;
pub(crate) use host_admission::RegisteredReadAdmission;
pub(crate) use pinned_read::{PinnedFileIdentity, PinnedReadOnlyRoot};
#[cfg(test)]
mod fault_matrix;
mod identity;
mod journal;
mod local_create;
#[cfg(test)]
mod native_existing_replace_probe;
#[cfg(all(test, windows))]
mod native_windows_replace_probe;
mod local_recovery;
mod recovery_review;
/// Crypto envelope contract only: no production key custody or archive writer yet.
pub mod sealed_archive;
mod recovery_dashboard;
mod planner;
mod protection_store;
#[cfg(feature = "sync-s3")]
mod s3_preimage_capture;
#[cfg(feature = "sync-s3")]
mod s3_reserved_replace;
#[cfg(feature = "sync-s3")]
mod profile_binding;
mod path;
mod pinned_read;
#[cfg(windows)]
mod filegate_windows_acl;
#[cfg(windows)]
mod pinned_windows;
mod protocol;
mod provider;
mod s3;
mod s3_manifest;
// Inert V2 contract only: no S3 pack writer, reader or auto-sync activation.
mod s3_pack_index;
// Gated pure verifier: no S3 transport, key custody, file changes, or Sync.
#[cfg(feature = "signed-vault-root-lab")]
mod s3_signed_root_paev1;
// Never part of the installed application or an S3 permission grant.
#[cfg(feature = "signed-vault-policy-lab")]
mod s3_publisher_policy_lab;
// E3a never reads the cloud or enrolls a Vault; read-only join preview only.
#[cfg(feature = "signed-vault-pairing-lab")]
mod s3_join_genesis_lab;
// E3b: opt-in read-only pinned V2 join inspection, not default Akula Sync.
#[cfg(feature = "signed-vault-pinned-join-lab")]
mod s3_join_pinned_lab;
mod s3_manifest_delta;
mod s3_coord_v2;
#[cfg(feature = "sync-s3")]
mod s3_anchored_delta;
#[cfg(feature = "sync-s3")]
mod s3_delta_walker;
#[cfg(feature = "sync-s3")]
mod s3_remote_anchor;
// App-data-only V2 trust checkpoints: explicit genesis pin and monotonic
// remote observation, never a per-document or user-write baseline.
#[cfg(feature = "sync-s3")]
mod s3_v2_trust_anchor;
#[cfg(feature = "sync-s3")]
mod s3_head_probe;
#[cfg(feature = "sync-s3")]
mod s3_transport;
#[cfg(feature = "sync-s3")]
mod yandex_readonly_probe;
mod upload;
mod vault;

pub use core_scan::{CoreVaultScanner, LocalScanPreview, LocalScanTrust, ScanLimits};
pub use dag::{DagError, IngestOutcome, RevisionDag, RevisionOperation, RevisionRecord};
pub use delta::{plan_revision_exchange, DeltaPlanError, MissingRevisionBatch, RevisionExchangePlan, RevisionInventory};
pub use dag_store::{DagStoreError, DagStoreResult, SqliteDagStore};
pub use recovery_dashboard::{
    native_recovery_overview, native_recovery_case_preview, native_record_recovery_decision,
    NativeRecoveryOverview, NativeRecoveryExample, NativeRecoveryCasePreview,
    NativeRecoveryDecision, NativeRecoveryReviewError, NativeRecoveryReviewReceipt,
    RECOVERY_JOURNAL_FILENAME,
};
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
pub use s3::{S3ContractError, S3HeadPointerV1, S3Keyspace};
pub use s3_coord_v2::{
    S3DeltaEnvelopeV2, S3DeltaEnvelopeV2Error, S3DeltaEnvelopeCoverageV2,
    S3DeltaPackRefV2, MAX_DELTA_ENVELOPE_BYTES, MAX_DELTA_ENVELOPE_REVISIONS,
    MAX_DELTA_PACK_BYTES, MAX_DELTA_PACK_MEMBERS,
};
#[cfg(feature = "sync-s3")]
pub use s3_anchored_delta::{
    walk_s3_delta_from_saved_anchor, S3AnchoredWalkError,
};
#[cfg(feature = "sync-s3")]
pub use s3_remote_anchor::{
    S3RemoteAnchor, S3RemoteAnchorError, S3RemoteAnchorJournal,
    S3RemoteAnchorOutcome, S3RemoteAnchorScope,
};
#[cfg(feature = "signed-vault-policy-receipt-lab")]
pub use s3_v2_trust_anchor::{
    PolicyReceiptOutcome, V2PolicyReceiptError, V2TrustedPublisherPolicy,
};
#[cfg(feature = "signed-vault-atomic-join-lab")]
pub use s3_v2_trust_anchor::{AtomicJoinError, AtomicJoinOutcome};
#[cfg(feature = "signed-vault-signed-successor-lab")]
pub use s3_v2_trust_anchor::{SignedSuccessorError, SignedSuccessorOutcome};
#[cfg(feature = "signed-vault-receiver-session-lab")]
pub use s3_v2_trust_anchor::{ReceiverJoinSession, ReceiverJoinSessionError};
#[cfg(feature = "sync-s3")]
pub use s3_v2_trust_anchor::{
    S3V2TrustAnchorError, S3V2TrustAnchorOutcome,
    S3V2TrustedAnchor, S3V2TrustedAnchorJournal,
};
#[cfg(feature = "sync-s3")]
pub use s3_delta_walker::{
    walk_s3_manifest_delta, S3DeltaWalkCheckpoint, S3DeltaWalkError, S3DeltaWalkPreview,
};
pub use s3_manifest_delta::{
    inspect_s3_manifest_delta, S3ManifestDeltaBase, S3ManifestDeltaError,
    S3ManifestDeltaLimits, S3ManifestDeltaOutcome, S3ManifestShardDelta,
    S3ManifestSnapshotReason,
};
pub use s3_manifest::{
    S3ManifestError, S3ManifestShardRefV1, S3ManifestShardV1, S3ManifestV1,
    S3RevisionRefV1, MAX_MANIFEST_BYTES, MAX_REVISIONS, MAX_SHARD_BYTES,
    MAX_SHARD_ENTRIES, MAX_SHARDS,
};
#[cfg(feature = "signed-vault-root-lab")]
pub use s3_signed_root_paev1::{
    verify_signed_root, ApprovedPublisherKeys, ApprovedV2RootScope,
    DecodedDsseEnvelope, DecodedSignature, SignedRootError,
    SignedRootObservation, ROOT_PAYLOAD_TYPE,
};
#[cfg(feature = "signed-vault-pairing-lab")]
pub use s3_join_genesis_lab::{
    review_offline_join_genesis, IndependentlyApprovedJoin, JoinGenesisError,
    JoinGenesisReview, JOIN_GENESIS_TYPE,
};
#[cfg(feature = "signed-vault-pinned-join-lab")]
pub use s3_join_pinned_lab::{
    inspect_pinned_join_receipt, preview_yandex_join_genesis_read_only,
    PinnedJoinError, PinnedJoinReadOnlyObservation,
};
#[cfg(feature = "signed-vault-policy-lab")]
pub use s3_publisher_policy_lab::{
    verify_policy_transition, PolicyLabError, PolicyTransitionObservation,
    PublisherPolicyV1, POLICY_PAYLOAD_TYPE,
};
pub use s3_pack_index::{
    S3PackedHeadV2, S3PackIndexRootV2, S3PackIndexShardV2,
    S3PackDescriptorV2, S3PackMemberV2, S3PackShardRefV2,
    S3PackIndexV2Error, MAX_PACK_V2_ROOT_BYTES,
    MAX_PACK_V2_INDEX_BYTES, MAX_PACK_V2_SHARD_BYTES,
    MAX_PACK_V2_SHARDS, MAX_PACK_V2_PACKS_PER_SHARD,
    MAX_PACK_V2_MEMBERS_PER_PACK, MAX_PACK_V2_BYTES,
};
#[cfg(feature = "sync-s3")]
pub use s3_head_probe::{
    observe_s3_current_head, read_s3_digest_matched_manifest, S3DigestMatchedManifest,
    S3HeadReadError, S3ObservedHead,
};
#[cfg(feature = "sync-s3")]
pub use yandex_readonly_probe::{probe_yandex_s3_read_only, YandexS3ProbeError};
#[cfg(feature = "sync-s3")]
pub use s3_transport::{
    AwsS3ProbeClient, S3BoundedObject, S3ConnectionProfile, S3DiagnosticPage,
    S3AwsAccountProofError, S3AwsVerifiedAccountScope,
    S3ObservedObject, S3PutCondition, S3TransportError,
    S3MultipartError, S3MultipartIntent, S3MultipartJournal, S3MultipartOutcome,
    S3MultipartPhase, S3MultipartStatus, S3StreamDigest,
    S3HeadAttemptPhase, S3HeadError, S3HeadJournal, S3HeadPublication, S3VerifiedHead,
    S3PinnedRevisionPage, S3PinnedRevisionSnapshot,
    S3VerifiedRevisionMetadataStore, MAX_S3_REVISION_METADATA_CACHE_BYTES,
    S3SelectiveReadError, S3SelectiveRevisionPreview,
    S3CoordSidecarReadError, S3CoordSidecarPreview,
    S3PinnedV2ReadError, S3PinnedV2ReadPreview,
    S3CoordOfflineChainError, S3CoordOfflineChainPreview,
    S3CoordSelectivePacksPreview,
    S3VerifiedHintsError, S3VerifiedHintsJournal,
    plan_verified_s3_revision_replay, S3CasReplanPreview, S3ReplanError,
};

pub use upload::{upload_immutable_verified, UploadOutcome, VerifiedUploadError};
pub use vault::{
    LocalFileEntry, LocalFileSnapshot, PreservationReceipt, VaultFileKind, VaultPort,
    VaultPortError,
};

/// No sync scheduler or external I/O is started by importing these contracts.
/// Runtime activation/UI remain out of scope until the S3 slice is validated.
pub const SYNC_ENGINE_FEATURE_ENABLED: bool = cfg!(feature = "sync-engine");
/// Local custody checks can compile without an S3 SDK or sync scheduler.
pub const HOST_CUSTODY_FEATURE_ENABLED: bool = cfg!(feature = "host-custody");

#[cfg(all(test, not(feature = "sync-engine")))]
mod disabled_tests {
    #[test]
    fn compiled_contracts_do_not_enable_cloud_sync_by_default() {
        assert!(!super::SYNC_ENGINE_FEATURE_ENABLED);
    }
}
