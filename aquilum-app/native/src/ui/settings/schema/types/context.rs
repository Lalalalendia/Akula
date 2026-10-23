use std::path::PathBuf;
use aquilum_core::mcp::McpStatus;
use aquilum_core::sync::{NativeRecoveryOverview, NativeRecoveryCasePreview};

pub struct Context {
    pub workspace: Option<PathBuf>,
    pub scale: f64,
    pub home_page: String,
    pub mcp: Option<McpStatus>,
    pub sync_recovery: Option<NativeRecoveryOverview>,
    #[cfg(feature = "s3-e2e01-review-preview")]
    pub s3_bound_publications: Option<NativeRecoveryOverview>,
    pub sync_selected: Option<NativeRecoveryCasePreview>,
    pub sync_notice: Option<String>,
    #[cfg(feature = "dropbox-readonly-preview")]
    pub dropbox_preview_status: String,
    #[cfg(feature = "dropbox-readonly-preview")]
    pub dropbox_preview_busy: bool,
    pub copied: Option<usize>,
    pub trash_page: usize,
    pub autostart: Option<bool>,
    pub update: Option<crate::updater::Status>,
}

impl Context {
    #[cfg(test)]
    pub fn new(scale: f64) -> Self {
        Context { workspace: None, scale, home_page: String::new(), mcp: None, sync_recovery: None, #[cfg(feature = "s3-e2e01-review-preview")] s3_bound_publications: None, sync_selected: None, sync_notice: None, #[cfg(feature = "dropbox-readonly-preview")] dropbox_preview_status: "DISCONNECTED / SYNC_DISABLED".into(), #[cfg(feature = "dropbox-readonly-preview")] dropbox_preview_busy: false, copied: None, trash_page: 0, autostart: None, update: None }
    }
}
