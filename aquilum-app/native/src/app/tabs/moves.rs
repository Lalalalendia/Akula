use std::path::{Path, PathBuf};

use aquilum_core::search::paths::same_path;
use masonry::app::RenderRoot;

use super::super::{App, Popup};
use crate::i18n::{t, t_with};
use crate::ui::dialog::ConfirmDialog;
use masonry::kurbo::Point;

impl App {
    pub(in crate::app) fn follow_move(&mut self, root: &mut RenderRoot, from: &Path, to: &Path) {
        self.interface.move_metadata(from, to);
        if let Some(note) = self.note.as_mut().filter(|n| n.is(from)) {
            note.path = to.to_path_buf();
            self.workspace.core.active_note.set(Some(note.path.clone()));
            self.tree.set_active(root, Some(note.path.clone()));
            self.refresh_note_names(root);
        }
        self.relocate_tabs(root, from, to);
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
            note.save_error.clone()
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

    pub(in crate::app) fn relocate_tabs(&mut self, root: &mut RenderRoot, from: &Path, to: &Path) {
        if !self.tabs.rename(from, to) {
            return;
        }
        if let Some(session) = &self.session {
            for tab in self.tabs.tabs().iter().filter(|t| t.path.as_deref() == Some(to)) {
                session.renamed(&self.workspace.core, tab);
            }
        }
        self.refresh_tabs(root);
    }

    pub(in crate::app) fn close_missing_tabs(&mut self, root: &mut RenderRoot, removed: &[PathBuf]) -> bool {
        let inside = |path: &Path| path.ancestors().any(|ancestor| removed.iter().any(|gone| same_path(ancestor, gone)));
        let gone: Vec<_> = self.tabs.tabs().iter().filter(|tab| tab.path.as_ref().is_some_and(|p| !p.exists() && inside(p))).map(|tab| tab.id).collect();
        for id in &gone {
            self.tabs.close(*id);
        }
        if !gone.is_empty() {
            self.show_active(root);
        }
        !gone.is_empty()
    }
}
