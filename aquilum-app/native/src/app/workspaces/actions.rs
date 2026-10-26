use super::*;

impl App {
    pub(in crate::app) fn on_workspaces_action(&mut self, root: &mut RenderRoot, id: WidgetId, action: &ErasedAction) -> Outcome {
        let Some(Popup::Workspaces(mut state)) = self.popup.take() else { return Outcome::default() };
        if let Some((forget, workspace, was_current)) = state.forget.take() {
            let Some(answer) = forget.answer(id, action) else {
                state.forget = Some((forget, workspace, was_current));
                self.popup = Some(Popup::Workspaces(state));
                return Outcome::default();
            };
            crate::ui::dialog::close(root, forget.layer);
            if answer == DialogAnswer::Cancel {
                root.focus_on(Some(state.dialog.layer));
                self.popup = Some(Popup::Workspaces(state));
                return Outcome::default();
            }
            root.remove_layer(state.dialog.layer);
            if was_current {
                let outcome = self.close_current_workspace(root);
                if self.tree.root().is_some() {
                    self.show_workspaces(root, Some(t("workspaces.closeSaveFailed")));
                    return outcome;
                }
            }
            let error = self.workspace.core.ui_state.forget_workspace(workspace).err().map(|e| {
                report!("база не забыта: {e:?}");
                t("workspaces.forgetFailed")
            });
            self.show_workspaces(root, error);
            return Outcome { title: was_current, ..Outcome::default() };
        }
        let dialog = &state.dialog;
        let pressed = action.downcast_ref::<Pressed>().is_some();
        let closing = (id == dialog.layer && matches!(action.downcast_ref::<DialogAnswer>(), Some(DialogAnswer::Cancel)))
            || (id == dialog.close && pressed);
        if closing {
            crate::ui::dialog::close(root, dialog.layer);
            return Outcome::default();
        }
        if Some(id) == dialog.close_active && pressed {
            root.remove_layer(state.dialog.layer);
            let outcome = self.close_current_workspace(root);
            if self.tree.root().is_some() {
                self.show_workspaces(root, Some(t("workspaces.closeSaveFailed")));
            }
            return outcome;
        }
        if id == dialog.open_folder && pressed {
            self.popup = Some(Popup::Workspaces(state));
            return self.pick_workspace(root);
        }
        let row = dialog.rows.iter().find(|(row, _, _)| *row == id).map(|(_, path, uuid)| (path.clone(), *uuid));
        match (row, action.downcast_ref::<RowAction>()) {
            (Some((path, _)), Some(RowAction::Open)) => {
                self.popup = Some(Popup::Workspaces(state));
                return self.switch_workspace(root, path);
            }
            (Some((path, workspace)), Some(RowAction::Forget)) => {
                let (title, description) = workspaces::forget_texts(&path);
                let (layer, forget) = ConfirmDialog::new(&title, &description, &t("common.cancel"), &t("workspaces.forgetConfirm"));
                root.add_layer(layer, Point::ORIGIN);
                root.focus_on(Some(forget.layer));
                let was_current = self.tree.root().is_some_and(|root| workspaces::same_path(root, &path));
                state.forget = Some((forget, workspace, was_current));
            }
            _ => {}
        }
        self.popup = Some(Popup::Workspaces(state));
        Outcome::default()
    }
}
