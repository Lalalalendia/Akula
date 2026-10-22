use aquilum_core::sync::NativeRecoveryDecision;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
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
