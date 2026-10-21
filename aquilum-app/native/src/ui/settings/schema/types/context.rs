use super::*;
use std::path::PathBuf;
use aquilum_core::mcp::McpStatus;
use aquilum_core::sync::{NativeRecoveryOverview, NativeRecoveryCasePreview};

pub struct Context {
    pub workspace: Option<PathBuf>,
    pub scale: f64,
    pub home_page: String,
    pub mcp: Option<McpStatus>,
    pub sync_recovery: Option<NativeRecoveryOverview>,
    pub sync_selected: Option<NativeRecoveryCasePreview>,
    pub sync_notice: Option<String>,
    pub yandex_keys: Option<bool>,
    pub yandex_probe: crate::yandex_probe_session::ProbeView,
    pub yandex_probe_busy: bool,
    pub yandex_notice: Option<String>,
    pub copied: Option<usize>,
    pub trash_page: usize,
    pub autostart: Option<bool>,
    pub update: Option<crate::updater::Status>,
}

impl Context {
    #[cfg(test)]
    pub fn new(scale: f64) -> Self {
        Context { workspace: None, scale, home_page: String::new(), mcp: None, sync_recovery: None, sync_selected: None, sync_notice: None, yandex_keys: None, yandex_probe: crate::yandex_probe_session::ProbeView::NotChecked, yandex_probe_busy: false, yandex_notice: None, copied: None, trash_page: 0, autostart: None, update: None }
    }
}

