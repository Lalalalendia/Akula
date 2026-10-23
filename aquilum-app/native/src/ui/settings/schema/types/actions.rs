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
    CheckUpdates,
    InstallUpdate,
    PluginDemoInstall,
    PluginDemoRun,
    PluginDemoDisable,
    PluginDemoEnable,
    PluginDemoRemove,
}
