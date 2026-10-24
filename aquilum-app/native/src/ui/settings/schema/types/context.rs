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
    pub sync_notice: Option<String>, #[cfg(feature = "dropbox-readonly-preview")] pub dropbox_preview_status: String, #[cfg(feature = "dropbox-readonly-preview")] pub dropbox_preview_busy: bool,
    pub yandex_keys: Option<bool>, #[cfg(feature = "notion-offline-preview")] pub notion_offline: crate::notion_offline::NotionOfflineView,
    pub yandex_probe: crate::yandex_probe_session::ProbeView,
    pub yandex_probe_busy: bool,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    pub s3_pair: crate::s3_pair_session::PairUiView,
    pub yandex_notice: Option<String>,
    pub copied: Option<usize>,
    pub trash_page: usize,
    pub autostart: Option<bool>,
    pub update: Option<crate::updater::Status>,
}
#[cfg(test)]
mod defaults;
