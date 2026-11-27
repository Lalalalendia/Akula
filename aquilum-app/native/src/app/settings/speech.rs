mod poll;
use super::*;
use crate::i18n::{t, t_with};
use crate::workspace::speech_jobs::{Action as Speech, Command};

impl App {
    pub(super) fn press_speech(&self, action: Speech, section: Section) -> Effect {
        if section != Section::System {
            return Effect::None;
        }
        let dispatcher = self.workspace.speech();
        let command = match action {
            Speech::PickTool(tool) => rfd::FileDialog::new()
                .set_title(t_with(
                    "settings.system.speech.toolPicker",
                    &[("tool", tool.label())],
                ))
                .pick_file()
                .map(|path| Command::SetTool(tool, path)),
            Speech::PickMedia => rfd::FileDialog::new()
                .set_title(t("settings.system.speech.mediaPicker"))
                .pick_file()
                .map(Command::Inspect),
            Speech::Export(id) => rfd::FileDialog::new()
                .set_title(t("settings.system.speech.export"))
                .add_filter("WebVTT", &["vtt"])
                .set_file_name("subtitles.vtt")
                .save_file()
                .map(|path| Command::Export(id, path)),
            action => Some(Command::Action(action)),
        };
        if let Some(command) = command {
            dispatcher.send(command);
        }
        Effect::None
    }
}
