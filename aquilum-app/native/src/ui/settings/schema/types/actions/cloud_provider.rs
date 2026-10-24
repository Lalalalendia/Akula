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
    #[cfg(feature = "notion-offline-preview")]
    Notion,
}
impl CloudProvider {
    #[cfg(not(feature = "notion-offline-preview"))]
    pub const ALL: [Self; 7] = [
        Self::Yandex, Self::AmazonS3, Self::Minio, Self::S3Compatible,
        Self::GoogleDrive, Self::Dropbox, Self::WebDav,
    ];
    #[cfg(feature = "notion-offline-preview")]
    pub const ALL: [Self; 8] = [
        Self::Yandex,
        Self::AmazonS3,
        Self::Minio,
        Self::S3Compatible,
        Self::GoogleDrive,
        Self::Dropbox,
        Self::WebDav,
        Self::Notion,
    ];
    pub fn label(self) -> String {
        let key = match self {
            Self::Yandex => "settings.cloud.yandex",
            Self::AmazonS3 => "settings.cloud.amazon",
            Self::Minio => "settings.cloud.minio",
            Self::S3Compatible => "settings.cloud.otherS3",
            Self::GoogleDrive => "settings.cloud.googleDrive",
            Self::Dropbox => "settings.cloud.dropbox",
            Self::WebDav => "settings.cloud.webdav",
            #[cfg(feature = "notion-offline-preview")]
            Self::Notion => return "Notion".to_string(),
        };
        crate::i18n::t(key)
    }
}
