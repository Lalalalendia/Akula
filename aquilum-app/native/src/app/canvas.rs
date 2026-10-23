//! Native Canvas tab session, actions and guarded persistence.
//! One session per path survives tab reconstruction; File Gate owns durability.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use aquilum_core::files::structured::session::CanvasSession;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

use super::App;
use crate::ui::canvas::{CanvasAction, CanvasPage};
use crate::ui::handle::Handle;
use crate::ui::widgets::Slot;

#[derive(Default)]
pub(super) struct CanvasState {
    documents: HashMap<PathBuf, CanvasSession>,
    view: Option<Handle<CanvasPage>>,
    notice: Option<String>,
}

impl App {
    pub(in crate::app) fn canvas_can_leave_workspace(&mut self, root: &mut RenderRoot) -> bool {
        let pending = self.canvas.documents.iter()
            .find(|(_, session)| session.dirty())
            .map(|(path, _)| path.clone());
        let Some(path) = pending else { return true };
        self.close_workspaces(root);
        self.open(root, path);
        self.canvas_notify(root, "Canvas has unsaved edits. Resolve the conflict or Undo before changing vault.");
        false
    }

    pub(in crate::app) fn show_canvas_tab(&mut self, root: &mut RenderRoot, path: &Path) -> Result<(), String> {
        // Core StructuredDocument requires the caller to authorize the path.
        // Canonicalize both sides so symlinks/junctions cannot read outside
        // the selected local vault through a .canvas tab.
        let vault = self.tree.root().ok_or("Canvas requires an open vault")?;
        let scope = std::fs::canonicalize(vault).map_err(|e| e.to_string())?;
        let file = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
        if !file.starts_with(&scope) {
            return Err("Canvas is outside the active vault".to_owned());
        }
        if let Some(existing) = self.canvas.documents.get_mut(path) {
            existing.refresh_if_clean().map_err(|e| e.to_string())?;
        } else {
            let session = CanvasSession::open(&file).map_err(|e| e.to_string())?;
            self.canvas.documents.insert(path.to_path_buf(), session);
        }
        let session = &self.canvas.documents[path];
        let page = CanvasPage::editing(
            session.value(), session.dirty(), session.can_undo(), session.can_redo(),
            self.canvas.notice.clone(),
        );
        self.canvas.view = Some(Handle::of(&page));
        if let Some(chrome) = &self.chrome {
            chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
        }
        Ok(())
    }

    pub(in crate::app) fn on_canvas_action(
        &mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction,
    ) -> bool {
        let Some(op) = action.downcast_ref::<CanvasAction>() else { return false };
        let Some(view) = self.canvas.view.filter(|handle| handle.id() == id) else { return false };
        let Some(path) = self.tabs.active().path.as_ref() else { return false };
        let Some(session) = self.canvas.documents.get_mut(path) else { return false };

        let op_result = match op {
            CanvasAction::AddText(x, y) => session.add_text("New card", *x, *y).map(|_| true),
            CanvasAction::MoveNode(node, x, y) => session.move_node(node, *x, *y),
            CanvasAction::Undo => session.undo(),
            CanvasAction::Redo => session.redo(),
            CanvasAction::Save => Ok(false),
        };
        self.canvas.notice = match op_result {
            Err(err) => Some(err.to_string()),
            Ok(_) => session.save(&self.workspace.core).err().map(|e| format!("Not saved: {e}")),
        };
        let session = &self.canvas.documents[path];
        let value = session.value().clone();
        let (dirty, undo, redo) = (session.dirty(), session.can_undo(), session.can_redo());
        let notice = self.canvas.notice.clone();
        view.edit(root, |mut widget| {
            CanvasPage::update_state(&mut widget, &value, dirty, undo, redo, notice);
        });
        true
    }

    pub(in crate::app) fn canvas_dirty_active(&self) -> bool {
        self.tabs.active().path.as_ref()
            .and_then(|path| self.canvas.documents.get(path))
            .is_some_and(CanvasSession::dirty)
    }

    pub(in crate::app) fn canvas_notify(&mut self, root: &mut RenderRoot, notice: &str) {
        self.canvas.notice = Some(notice.to_owned());
        let Some(path) = self.tabs.active().path.as_ref() else { return };
        let Some(session) = self.canvas.documents.get(path) else { return };
        let Some(view) = self.canvas.view else { return };
        let value = session.value().clone();
        let (dirty, undo, redo) = (session.dirty(), session.can_undo(), session.can_redo());
        let notice = self.canvas.notice.clone();
        view.edit(root, |mut widget| {
            CanvasPage::update_state(&mut widget, &value, dirty, undo, redo, notice);
        });
    }

    #[cfg(test)]
    pub(in crate::app) fn canvas_view_id(&self) -> Option<WidgetId> {
        self.canvas.view.map(Handle::id)
    }

    pub(in crate::app) fn flush_canvas_on_shutdown(&mut self) {
        for (path, session) in &mut self.canvas.documents {
            if session.dirty() {
                if let Err(err) = session.save(&self.workspace.core) {
                    // Never overwrite the external version on a File Gate conflict.
                    eprintln!("Canvas not saved on shutdown ({}): {err}", path.display());
                }
            }
        }
    }
}
