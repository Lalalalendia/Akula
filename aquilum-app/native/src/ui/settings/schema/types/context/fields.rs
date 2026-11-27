use std::path::PathBuf; use aquilum_core::{mcp::McpStatus, sync::{NativeRecoveryOverview, NativeRecoveryCasePreview}};
pub struct Context {
    pub workspace: Option<PathBuf>,
    pub(crate) speech: Option<crate::workspace::speech_jobs::View>,
    pub scale: f64,
    pub home_page: String,
    pub mcp: Option<McpStatus>,
    pub sync_recovery: Option<NativeRecoveryOverview>,
    #[cfg(feature = "s3-e2e01-review-preview")]
    pub s3_bound_publications: Option<NativeRecoveryOverview>,
    pub sync_selected: Option<NativeRecoveryCasePreview>,
    pub sync_notice: Option<String>,
    #[cfg(feature = "yandex-disk-readonly-preview")] pub yandex_disk_status: String, #[cfg(feature = "yandex-disk-readonly-preview")] pub yandex_disk_busy: bool, #[cfg(feature = "yandex-disk-readonly-preview")] pub yandex_disk_review_account: Option<String>,
    #[cfg(feature = "yandex-disk-readonly-preview")] pub yandex_disk_app_configured: bool, #[cfg(feature = "yandex-disk-readonly-preview")] pub yandex_disk_vault_ready: bool, #[cfg(feature = "yandex-disk-readonly-preview")] pub yandex_disk_saved_account: Option<String>,
    #[cfg(feature = "dropbox-readonly-preview")] pub dropbox_preview_status: String,
    #[cfg(feature = "dropbox-readonly-preview")] pub dropbox_preview_busy: bool,
    pub server_probe: Option<super::super::ServerProbeView>,
    pub server_advanced: bool,
    pub yandex_keys: Option<bool>,
    pub yandex_probe: crate::yandex_probe_session::ProbeView,
    pub yandex_probe_busy: bool,
    #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
    pub s3_pair: crate::s3_pair_session::PairUiView,
    pub yandex_notice: Option<String>,
    pub copied: Option<usize>,
    pub trash_page: usize,
    pub autostart: Option<bool>,
    pub update: Option<crate::updater::Status>, #[cfg(windows)] pub optional_ytdlp: crate::video::optional_ytdlp::Status, #[cfg(windows)] pub optional_quickjs: crate::video::optional_ytdlp::Status,
}
