use super::Context;

impl Context {
    pub fn new(scale: f64) -> Self {
        Context { workspace: None, scale, home_page: String::new(), mcp: None, sync_recovery: None, #[cfg(feature = "s3-e2e01-review-preview")] s3_bound_publications: None, sync_selected: None, sync_notice: None, yandex_keys: None, yandex_probe: crate::yandex_probe_session::ProbeView::NotChecked, yandex_probe_busy: false, yandex_notice: None, #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))] s3_pair: crate::s3_pair_session::PairUiView::default(), copied: None, trash_page: 0, autostart: None, update: None }
    }
}
