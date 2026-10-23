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
        // Read-only journal inspection never implies a synchronized state.
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
            #[cfg(feature = "s3-e2e01-review-preview")]
            s3_bound_publications,
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
            server_probe: self.server_probe_view(section),
            server_advanced: false,
            sync_selected: None,
            sync_notice: None,
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
