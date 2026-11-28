use aquilum_core::sync::NativeRecoveryDecision; mod cloud_provider;
pub use cloud_provider::CloudProvider;
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenCloudProvider(CloudProvider),
    Speech(crate::workspace::speech_jobs::Action),
    #[cfg(feature = "dropbox-readonly-preview")] DropboxLoginAccount, #[cfg(feature = "dropbox-readonly-preview")] DropboxResumeAccount,
    #[cfg(feature = "dropbox-readonly-preview")] DropboxForgetAccount,
    #[cfg(feature = "dropbox-readonly-preview")] DropboxConnectReadOnly,
    #[cfg(feature = "dropbox-readonly-preview")] DropboxResumeReadOnly,
    #[cfg(feature = "yandex-disk-readonly-preview")] YandexDiskLoginReadOnly, #[cfg(feature = "yandex-disk-readonly-preview")] YandexDiskConfirmAccount, #[cfg(feature = "yandex-disk-readonly-preview")] YandexDiskInspectLocal, #[cfg(feature = "yandex-disk-readonly-preview")] YandexDiskForgetLocal,
    InspectRecovery(String),
    RequestRecoveryReview(NativeRecoveryDecision),
    OpenTrash,
    Restore(String), TrashPage(usize), CreateTemplates,
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
    #[cfg(windows)] InstallYtDlp, #[cfg(windows)] RemoveYtDlp, #[cfg(windows)] RefreshYtDlp, #[cfg(windows)] InstallQuickJs, #[cfg(windows)] RemoveQuickJs, #[cfg(windows)] RefreshQuickJs,
}
