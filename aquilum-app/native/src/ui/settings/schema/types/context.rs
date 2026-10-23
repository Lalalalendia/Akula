use super::*;

/// Session-only result; never persisted as installation approval or a
/// trusted server identity. Stale replies are discarded if any input changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerProbeView {
    Checking,
    Observed { facts: String },
    Unsupported { facts: String },
    Failed,
}

pub struct Context {
    pub server_probe: Option<ServerProbeView>,
    pub server_advanced: bool,
    pub workspace: Option<PathBuf>,
    pub scale: f64,
    pub home_page: String,
    pub mcp: Option<McpStatus>,
    pub sync_recovery: Option<NativeRecoveryOverview>,
    pub sync_selected: Option<NativeRecoveryCasePreview>,
    pub sync_notice: Option<String>,
    pub copied: Option<usize>,
    pub trash_page: usize,
    pub autostart: Option<bool>,
    pub update: Option<crate::updater::Status>,
}

impl Context {
    #[cfg(test)]
    pub fn new(scale: f64) -> Self {
        Context { server_probe: None, server_advanced: false, workspace: None, scale, home_page: String::new(), mcp: None, sync_recovery: None, sync_selected: None, sync_notice: None, copied: None, trash_page: 0, autostart: None, update: None }
    }
}

