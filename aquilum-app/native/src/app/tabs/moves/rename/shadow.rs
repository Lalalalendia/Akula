use super::{App, Path};
use crate::i18n::{t, t_with};

impl App {
    /// Hold the cross-process recovery publication lease until File Gate returns.
    /// If a writer is busy, reject immediately instead of freezing the UI.
    pub(super) fn guarded_rename_shadow_check(&self, from: &Path) -> Result<std::fs::File, String> {
        let workspace = self.session.as_ref()
            .map(|session| session.workspace_id)
            .ok_or_else(|| t_with(
                "editor.renameRecoveryCheck",
                &[("reason", "workspace recovery context unavailable")],
            ))?;
        let guard = crate::note::bridge::acquire_move_guard(&self.workspace.data_dir, workspace)
            .map_err(|error| t_with(
                "editor.renameRecoveryCheck",
                &[("reason", &error.to_string())],
            ))?;
        if let Some(reason) = self.rename_shadow_blocker(from) {
            return Err(reason);
        }
        Ok(guard)
    }

    /// A detached tab may retain an unreviewed shadow under its old filename.
    /// Do not move its file or parent folder before explicit recovery review.
    pub(super) fn rename_shadow_blocker(&self, from: &Path) -> Option<String> {
        let descendants = self.tabs.tabs().iter().filter_map(|tab| tab.path.as_deref())
            .filter(|path| path.starts_with(from));
        for path in std::iter::once(from).chain(descendants) {
            if !path.is_file() {
                continue;
            }
            let (Some(vault), Some(session)) = (self.tree.root(), self.session.as_ref()) else {
                return Some(t_with("editor.renameRecoveryCheck", &[("reason", "workspace recovery context unavailable")]));
            };
            let unreviewed = match self.note.as_ref().filter(|note| note.is(path)) {
                Some(note) => note.has_unreviewed_move_shadow(&self.workspace.data_dir, vault, session.workspace_id),
                None => crate::note::bridge::inspect_for_recovery(
                    &self.workspace.data_dir, vault, session.workspace_id, path,
                ).map(|record| record.is_some_and(|record| !record.already_saved)),
            };
            match unreviewed {
                Ok(true) => return Some(t("editor.renameRecoveryHold")),
                Err(error) => return Some(t_with(
                    "editor.renameRecoveryCheck", &[("reason", &error.to_string())],
                )),
                Ok(false) => {}
            }
        }
        if from.is_dir() {
            let (Some(vault), Some(session)) = (self.tree.root(), self.session.as_ref()) else {
                return Some(t_with("editor.renameRecoveryCheck", &[("reason", "workspace recovery context unavailable")]));
            };
            match crate::note::bridge::has_unreviewed_shadow_under_move(
                &self.workspace.data_dir, vault, session.workspace_id, from,
            ) {
                Ok(true) => return Some(t("editor.renameRecoveryHold")),
                Err(error) => return Some(t_with(
                    "editor.renameRecoveryCheck", &[("reason", &error.to_string())],
                )),
                Ok(false) => {}
            }
        }
        None
    }
}
