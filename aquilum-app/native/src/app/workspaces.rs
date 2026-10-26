mod actions;
mod close;
mod switch;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
use masonry::kurbo::Point;
use uuid::Uuid;

use super::{App, Outcome, Popup};
use crate::i18n::{t, t_with};
use crate::session::Session;
use crate::tabs::Tabs;
use crate::ui::dialog::{ConfirmDialog, DialogAnswer, DialogIds};
use crate::ui::file_tree::FileTree;
use crate::ui::widgets::Pressed;
use crate::ui::workspaces::{self, RowAction, WorkspaceDialog};

pub(super) struct WorkspacesState {
    dialog: WorkspaceDialog,
    forget: Option<(DialogIds, Uuid, bool)>,
}

#[cfg(test)]
impl WorkspacesState {
    pub(super) fn dialog(&self) -> &WorkspaceDialog {
        &self.dialog
    }

    pub(super) fn forget_layer(&self) -> Option<WidgetId> {
        self.forget.as_ref().map(|(ids, _, _)| ids.layer)
    }
}


impl App {
    fn save_before_workspace_change(&mut self, root: &mut RenderRoot) -> bool {
        if let Some(note) = self.note.as_mut() {
            // Keep the Note and editor alive until the serial disk worker has
            // reported the result. Note::leave consumes this recovery state.
            note.flush(&self.workspace.core, &self.text);
            if note.save_error.is_some() {
                self.refresh_save_error(root);
                return false;
            }
        }
        true
    }

    pub(super) fn open_workspaces(&mut self, root: &mut RenderRoot) {
        self.show_workspaces(root, None);
    }

    fn show_workspaces(&mut self, root: &mut RenderRoot, error: Option<String>) {
        let (known, error) = match self.workspace.core.ui_state.list_workspaces() {
            Ok(known) => {
                let mut unique: Vec<(uuid::Uuid, String)> = Vec::new();
                for w in known {
                    let path = crate::workspace::normalize(std::path::Path::new(&w.path));
                    if !unique.iter().any(|(_, p)| workspaces::same_path(std::path::Path::new(p), &path)) {
                        unique.push((w.id, path.to_string_lossy().into_owned()));
                    }
                }
                (unique, error)
            }
            Err(e) => {
                report!("список баз: {e:?}");
                (Vec::new(), Some(t_with("workspaces.listFailed", &[("reason", &format!("{e:?}"))])))
            }
        };
        let (layer, dialog) = workspaces::dialog(&known, self.tree.root(), error.as_deref());
        root.add_layer(layer, Point::ORIGIN);
        root.focus_on(Some(dialog.layer));
        self.popup = Some(Popup::Workspaces(WorkspacesState { dialog, forget: None }));
    }

    fn close_workspaces(&mut self, root: &mut RenderRoot) {
        if let Some(Popup::Workspaces(state)) = self.popup.take() {
            if let Some((forget, _, _)) = state.forget {
                crate::ui::dialog::close(root, forget.layer);
            }
            crate::ui::dialog::close(root, state.dialog.layer);
        }
    }

    pub(super) fn pick_workspace(&mut self, _root: &mut RenderRoot) -> Outcome {
        let start = self.tree.root().and_then(Path::parent).map(Path::to_path_buf);
        let sender = self.picked_tx.clone();
        let wake = std::sync::Arc::clone(&self.workspace.wake);
        std::thread::spawn(move || {
            let mut picker = rfd::FileDialog::new();
            if let Some(start) = start {
                picker = picker.set_directory(start);
            }
            if let Some(path) = picker.pick_folder()
                && sender.send(super::book::Picked::Workspace(path)).is_ok()
            {
                wake();
            }
        });
        Outcome::default()
    }

}
