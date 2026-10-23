use std::path::PathBuf;
use aquilum_core::mcp::McpStatus;
use aquilum_core::sync::{NativeRecoveryOverview, NativeRecoveryCasePreview};

/// Ephemeral SSH verification result, not a Node install receipt.
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
    #[cfg(feature = "s3-e2e01-review-preview")]
    pub s3_bound_publications: Option<NativeRecoveryOverview>,
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
        Context { server_probe: None, server_advanced: false, workspace: None, scale, home_page: String::new(), mcp: None, sync_recovery: None, #[cfg(feature = "s3-e2e01-review-preview")] s3_bound_publications: None, sync_selected: None, sync_notice: None, copied: None, trash_page: 0, autostart: None, update: None }
    }
}
