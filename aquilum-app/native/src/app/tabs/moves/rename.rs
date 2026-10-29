use super::{App, Path, PathBuf, RenderRoot};
use crate::app::Popup;
use crate::i18n::{t, t_with};
use crate::ui::dialog::ConfirmDialog;
use masonry::kurbo::Point;

impl App {
    /// Rebase the active editor path immediately after a verified File Gate rename,
    /// including notes inside a renamed directory.
    pub(in crate::app) fn follow_move(&mut self, root: &mut RenderRoot, from: &Path, to: &Path) {
        self.interface.move_metadata(from, to);
        let active = self.note.as_ref().and_then(|note| {
            if note.is(from) {
                Some(to.to_path_buf())
            } else {
                note.path.strip_prefix(from).ok().map(|suffix| to.join(suffix))
            }
        });
        if let Some(path) = active {
            // SnapshotQueue keeps its original document path. Merely changing
            // Note.path would stage all future recovery drafts against a missing
            // pre-rename file. Rebind after the File Gate move is confirmed.
            if let Some(mut note) = self.note.take() {
                note.path = path.clone();
                if let (Some(vault), Some(session)) = (self.tree.root(), self.session.as_ref()) {
                    let vault = vault.to_path_buf();
                    let workspace = session.workspace_id;
                    let data = self.workspace.data_dir.clone();
                    match note.rebind_shadow_after_move(&data, &vault, workspace) {
                        Ok(()) => self.arm_note_recovery(&mut note),
                        Err(error) => {
                            report!("recovery snapshot did not follow moved note {}: {error}", path.display());
                            note.save_error = Some(t_with("editor.renameRecoveryCheck", &[("reason", &error.to_string())]));
                        }
                    }
                }
                self.note = Some(note);
            }
            self.workspace.core.active_note.set(Some(path.clone()));
            self.tree.set_active(root, Some(path));
            self.refresh_note_names(root);
            self.refresh_save_error(root);
        }
        self.relocate_tabs(root, from, to);
    }

    pub(in crate::app) fn relocate_tabs(&mut self, root: &mut RenderRoot, from: &Path, to: &Path) {
        if !self.tabs.rename(from, to) {
            return;
        }
        // A moved file cannot reuse a cache entry keyed to its old disk path.
        self.inactive_notes.clear();
        if let Some(session) = &self.session {
            for tab in self.tabs.tabs().iter().filter(|tab| {
                tab.path.as_deref().is_some_and(|path| path.starts_with(to))
            }) {
                session.renamed(&self.workspace.core, tab);
            }
        }
        self.refresh_tabs(root);
    }

    /// Keep the visible note on its original path until File Gate succeeds.
    /// This synchronous path is intentional until an async edit/navigation
    /// fence exists; changing the path before the disk operation loses notes.
    pub(in crate::app) fn rename_file(&mut self, root: &mut RenderRoot, from: PathBuf, to: PathBuf) {
        if from == to {
            return;
        }
        let save_error = if let Some(note) = self.note.as_mut().filter(|n| n.path.starts_with(&from)) {
            note.flush(&self.workspace.core, &self.text);
            // A successful save ACK queues exact shadow cleanup on this same
            // worker. Wait for that cleanup before checking outstanding drafts.
            crate::disk::wait();
            note.save_error.clone().or_else(|| note.shadow_error())
        } else {
            // Detached note saves also use the native disk worker.
            crate::disk::wait();
            None
        };
        if let Some(reason) = save_error {
            report!("{} не переименован: несохранённые изменения: {reason}", from.display());
            if self.note.as_ref().is_some_and(|note| note.path.starts_with(&from)) {
                self.refresh_note_names(root);
                self.refresh_save_error(root);
            }
            self.show_rename_error(root, &from, &to, &t_with("editor.renameNeedsSave", &[("reason", &reason)]));
            return;
        }

        // A retained crash draft belongs to the old file identity. Moving the
        // source now would make that backup undiscoverable by normal recovery.
        let shadow_blocker = self.note.as_ref().filter(|n| n.path.starts_with(&from)).and_then(|note| {
            let (Some(vault), Some(session)) = (self.tree.root(), self.session.as_ref()) else { return None };
            match note.has_unreviewed_move_shadow(&self.workspace.data_dir, vault, session.workspace_id) {
                Ok(true) => Some(t("editor.renameRecoveryHold")),
                Err(error) => Some(t_with("editor.renameRecoveryCheck", &[("reason", &error.to_string())])),
                Ok(false) => None,
            }
        });
        if let Some(reason) = shadow_blocker {
            report!("{} не переименован: {reason}", from.display());
            self.show_rename_error(root, &from, &to, &reason);
            return;
        }

        match aquilum_core::files::gate::rename(&self.workspace.core, &from, &to) {
            Ok(_) => {
                self.follow_move(root, &from, &to);
                self.tree.rename_succeeded(root, &from, &to);
            }
            Err(error) => {
                report!("{} не переименован: {error}", from.display());
                if self.note.as_ref().is_some_and(|note| note.path.starts_with(&from)) {
                    self.refresh_note_names(root);
                }
                self.tree.paths_changed(root, &[from.clone(), to.clone()]);
                self.show_rename_error(root, &from, &to, &error.to_string());
            }
        }
    }

    fn show_rename_error(&mut self, root: &mut RenderRoot, from: &Path, to: &Path, reason: &str) {
        if self.popup.is_some() {
            return;
        }
        let name = from.file_name().unwrap_or_default().to_string_lossy();
        let description = t_with("editor.renameFailed", &[("path", &name), ("reason", reason)]);
        let (layer, dialog) = ConfirmDialog::new(
            &t("editor.renameFailedTitle"),
            &description,
            &t("editor.close"),
            &t("editor.revealFile"),
        );
        let reveal = if from.exists() {
            from.to_path_buf()
        } else if to.exists() {
            to.to_path_buf()
        } else {
            from.parent().unwrap_or(from).to_path_buf()
        };
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::RenameFailed { dialog, reveal });
    }
}
