use aquilum_core::sync::NativeRecoveryDecision;
/// The catalog is provider discovery, not permission to use a provider.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloudProvider {
    Yandex,
    AmazonS3,
    Minio,
    S3Compatible,
    GoogleDrive,
    Dropbox,
    WebDav,
}
impl CloudProvider {
    pub const ALL: [Self; 7] = [
        Self::Yandex,
        Self::AmazonS3,
        Self::Minio,
        Self::S3Compatible,
        Self::GoogleDrive,
        Self::Dropbox,
        Self::WebDav,
    ];
    pub fn label(self) -> String {
        crate::i18n::t(match self {
            Self::Yandex => "settings.cloud.yandex",
            Self::AmazonS3 => "settings.cloud.amazon",
            Self::Minio => "settings.cloud.minio",
            Self::S3Compatible => "settings.cloud.otherS3",
            Self::GoogleDrive => "settings.cloud.googleDrive",
            Self::Dropbox => "settings.cloud.dropbox",
            Self::WebDav => "settings.cloud.webdav",
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenCloudProvider(CloudProvider),
    #[cfg(feature = "dropbox-readonly-preview")] DropboxLoginAccount,
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
