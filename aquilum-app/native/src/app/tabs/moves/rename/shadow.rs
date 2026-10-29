use super::{App, Path};
use crate::i18n::{t, t_with};

impl App {
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
        None
    }
}
