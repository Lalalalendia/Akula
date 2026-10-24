use aquilum_core::sync::NativeRecoveryDecision;
mod cloud_provider;
pub use cloud_provider::CloudProvider;
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenCloudProvider(CloudProvider),
    #[cfg(feature = "notion-offline-preview")] NotionOpenRoot(String),
    #[cfg(feature = "notion-offline-preview")] NotionConnect,
    #[cfg(feature = "notion-offline-preview")] NotionRefresh(String),
    #[cfg(feature = "notion-offline-preview")] NotionForget(String),
    #[cfg(feature = "notion-offline-preview")] NotionOpenPage(String),
    #[cfg(feature = "notion-offline-preview")] NotionPageBatch(usize),
    #[cfg(feature = "notion-offline-preview")] NotionTextChunk(usize),
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
    CheckUpdates,
    InstallUpdate,
}
