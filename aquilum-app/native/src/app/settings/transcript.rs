use super::{App, Effect, Outcome, Popup, SettingsDialog};
use crate::ui::settings::transcript::Editor;
use aquilum_core::background_jobs::JobId;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};
impl App {
    pub(super) fn open_transcript(
        &self,
        root: &mut RenderRoot,
        dialog: &mut SettingsDialog,
        job: JobId,
    ) -> Effect {
        let (layer, mut editor) = Editor::new(self.workspace.speech(), job);
        crate::app::ui_actions::mount_modal(root, layer, editor.layer());
        editor.poll(root);
        dialog.transcript = Some(editor);
        Effect::None
    }

    pub(super) fn on_transcript_action(
        &mut self,
        root: &mut RenderRoot,
        id: WidgetId,
        action: &ErasedAction,
        mut dialog: SettingsDialog,
    ) -> Outcome {
        if let Some(editor) = dialog.transcript.as_mut() {
            if editor.action(root, self.workspace.speech(), id, action) {
                root.remove_layer(editor.layer());
                root.focus_on(Some(dialog.layer));
                dialog.transcript = None;
            } else {
                editor.poll(root);
            }
        }
        self.popup = Some(Popup::Settings { dialog, menu: None });
        Outcome::default()
    }

    pub(super) fn poll_transcript(&mut self, root: &mut RenderRoot) {
        let editor = match &mut self.popup {
            Some(Popup::Settings { dialog, .. }) => dialog.transcript.as_mut(),
            Some(Popup::Transcript(editor)) => Some(editor.as_mut()),
            _ => None,
        };
        if let Some(editor) = editor { editor.poll(root); }
    }
}
