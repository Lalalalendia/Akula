use super::CloudProvider;
use aquilum_core::sync::NativeRecoveryDecision;
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenCloudProvider(CloudProvider),
    #[cfg(feature = "notion-offline-preview")] NotionOpenRoot(String),
    #[cfg(feature = "notion-offline-preview")] NotionBasesOpen(String),
    #[cfg(feature = "notion-offline-preview")] NotionBasesSelect(String, String),
    #[cfg(feature = "notion-offline-preview")] NotionBasesPage(usize, usize), #[cfg(feature = "notion-offline-preview")] NotionBasesSelectCell(String, String),
    #[cfg(feature = "notion-offline-preview")] NotionBasesChange(String, String, bool),
    #[cfg(feature = "notion-offline-preview")] NotionBasesReload, #[cfg(feature = "notion-offline-preview")] NotionBasesPreviewRebase(String), #[cfg(feature = "notion-offline-preview")] NotionBasesApplyRebase, #[cfg(feature = "notion-offline-preview")] NotionBasesToggleKeepLocal(String, String), #[cfg(feature = "notion-offline-preview")] NotionBasesToggleAcceptNotion(String, String), #[cfg(feature = "notion-offline-preview")] NotionBasesApplyReviewedRebase, #[cfg(feature = "notion-offline-preview")] NotionBasesBeginEdit(String, String), #[cfg(feature = "notion-offline-preview")] NotionBasesSaveDraft, #[cfg(feature = "notion-offline-preview")] NotionBasesCancelDraft, #[cfg(feature = "notion-offline-preview")] NotionBasesNewColumn(aquilum_core::notion_api_probe::bases_edits::LocalKind), #[cfg(feature = "notion-offline-preview")] NotionBasesRenameColumn(String), #[cfg(feature = "notion-offline-preview")] NotionBasesSaveSchema, #[cfg(feature = "notion-offline-preview")] NotionBasesCancelSchema,
    #[cfg(feature = "notion-offline-preview")] NotionConnect,
    #[cfg(feature = "notion-offline-preview")] NotionRefresh(String),
    #[cfg(feature = "notion-offline-preview")] NotionForget(String),
    #[cfg(feature = "notion-offline-preview")] NotionOpenPage(String),
    #[cfg(feature = "notion-offline-preview")] NotionPageBatch(usize),
    #[cfg(feature = "notion-offline-preview")] NotionTextChunk(usize), #[cfg(feature = "notion-offline-preview")] NotionReviewImport(String), #[cfg(feature = "notion-offline-preview")] NotionPublishImport(String),
    #[cfg(feature = "dropbox-readonly-preview")] DropboxLoginAccount, #[cfg(feature = "dropbox-readonly-preview")] DropboxResumeAccount,
    #[cfg(feature = "dropbox-readonly-preview")] DropboxForgetAccount,
    #[cfg(feature = "dropbox-readonly-preview")] DropboxConnectReadOnly,
    #[cfg(feature = "dropbox-readonly-preview")] DropboxResumeReadOnly,
    InspectRecovery(String),
    RequestRecoveryReview(NativeRecoveryDecision),
    OpenTrash,
    Restore(String),
    TrashPage(usize),
    CreateTemplates,
    NewToken,
    Copy(usize, String),
    OpenUrl(&'static str),
    StoreYandexKeys,
    RemoveYandexKeys,
    CheckYandexConnection,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    ConfigureS3Pair,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    ExportS3Pair,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    ImportS3Pair,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    SyncS3PairOnce,
    VerifyServer,
    ToggleServerAdvanced,
    InstallServer,
    CheckUpdates,
    InstallUpdate,
}
