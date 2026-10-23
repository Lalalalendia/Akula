use aquilum_core::sync::NativeRecoveryDecision;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Read-only SSH verification; never an install approval.
    VerifyServer,
    ToggleServerAdvanced,
    /// Disabled until signed installer and explicit owner approval exist.
    InstallServer,
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
}
