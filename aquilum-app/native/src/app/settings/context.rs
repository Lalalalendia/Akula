use super::*;
use std::path::Path;

impl App {
    pub(super) fn settings_context(&self, section: Section, copied: Option<usize>, trash_page: usize) -> Context {
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
        let server_probe = if section == Section::Server {
            let current = self.workspace.core.settings.get_config().server_setup;
            self.server_probe.observation.as_ref()
                .filter(|(draft, _)| draft == &current)
                .map(|(_, status)| status.clone())
        } else {
            None
        };
        Context {
            server_probe,
            server_advanced: false,
            workspace,
            sync_recovery,
            #[cfg(feature = "s3-e2e01-review-preview")]
            s3_bound_publications,
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
