use super::super::*;
use std::path::Path;
impl App {
    pub(in crate::app::settings) fn settings_context(&self, section: Section, copied: Option<usize>, trash_page: usize) -> Context {
        let workspace = self.tree.root().map(Path::to_path_buf);
        let home_page = workspace
            .as_ref()
            .and_then(|root| {
                let path = root.to_string_lossy();
                let known = self.workspace.core.ui_state.list_workspaces().ok()?;
                known.into_iter().find(|w| w.path == path).map(|w| w.home_page)
            })
            .unwrap_or_default();
        // SQLite is opened only for this page, and strictly read-only.
        // Missing/unsupported journals are NOT described as "synchronized".
        let sync_recovery = if section == Section::Sync {
            workspace.as_deref().map(|vault| {
                aquilum_core::sync::native_recovery_overview(
                    &self.workspace.core, &self.workspace.data_dir, vault,
                )
            })
        } else {
            None
        };
        #[cfg(feature = "s3-e2e01-review-preview")]
        let s3_bound_publications = if section == Section::Sync {
            workspace.as_deref().map(|root| {
                aquilum_core::sync::native_s3_bound_publication_overview(
                    &self.workspace.core, &self.workspace.data_dir, root,
                )
            })
        } else {
            None
        };
        let yandex_keys = if section == Section::Sync {
            let bucket = self.workspace.core.settings.get_config().s3_onboarding.bucket;
            crate::yandex_credentials::present(&bucket).ok()
        } else {
            None
        };
        let bucket = self.workspace.core.settings.get_config().s3_onboarding.bucket;
        Context {
            workspace: workspace.clone(),
            sync_recovery,
            #[cfg(feature = "yandex-disk-readonly-preview")] yandex_disk_status: self.yandex_disk_preview.status.to_owned(), #[cfg(feature = "yandex-disk-readonly-preview")] yandex_disk_busy: self.yandex_disk_preview.busy, #[cfg(feature = "yandex-disk-readonly-preview")] yandex_disk_review_account: self.yandex_disk_preview.review_account.clone(), #[cfg(feature = "yandex-disk-readonly-preview")] yandex_disk_app_configured: crate::app::yandex_disk_preview::YandexDiskPreview::has_registered_app(), #[cfg(feature = "yandex-disk-readonly-preview")] yandex_disk_vault_ready: self.yandex_disk_local_vault().is_some(),
            #[cfg(feature = "s3-e2e01-review-preview")] s3_bound_publications,
            yandex_keys,
            yandex_probe: self.yandex_probe.view_for(&bucket),
            yandex_probe_busy: self.yandex_probe.busy(),
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            s3_pair: crate::s3_pair_session::PairUiView {
                status: self.s3_pair.status_for(workspace.as_deref()),
                busy: self.s3_pair.busy(),
                verified: section == Section::Sync && self.s3_pair.verified_for(workspace.as_deref()),
                configured: section == Section::Sync && workspace.as_deref()
                    .and_then(|root| aquilum_core::sync::s3_pair_host::read_pair_profile(
                        root, &self.workspace.data_dir,
                    ).ok().flatten()).is_some(),
            },
            yandex_notice: None,
            server_probe: self.server_probe_view(section), server_advanced: false,
            sync_selected: None, sync_notice: None,
            #[cfg(feature = "dropbox-readonly-preview")] dropbox_preview_status: self.dropbox_preview.status.clone(), #[cfg(feature = "dropbox-readonly-preview")] dropbox_preview_busy: self.dropbox_preview.busy,
            scale: self.interface.scale(),
            home_page,
            mcp: Some(self.workspace.core.mcp.status()),
            copied,
            trash_page,
            autostart: crate::autostart::available().then(crate::autostart::enabled),
            update: crate::updater::available().then(|| self.updater.status()),
        }
    }
}
