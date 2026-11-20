use super::*;
use crate::i18n::t;

impl App {
    pub(super) fn press_video_tool(&mut self, action: Action, section: Section) -> Effect {
        if section != Section::System {
            return Effect::None;
        }
        match action {
            Action::InstallYtDlp => {
                let accepted = rfd::MessageDialog::new()
                    .set_title(t("settings.system.ytdlpConfirmTitle"))
                    .set_description(t("settings.system.ytdlpConfirmText"))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show() == rfd::MessageDialogResult::Yes;
                if accepted {
                    let wake = std::sync::Arc::clone(&self.workspace.wake);
                    if let Err(error) = crate::video::optional_ytdlp::start(self.workspace.data_dir.clone(), wake) {
                        report!("yt-dlp opt-in install refused: {error}");
                    }
                }
                Effect::Content
            }
            Action::RemoveYtDlp => {
                let accepted = rfd::MessageDialog::new()
                    .set_title(t("settings.system.ytdlpRemoveTitle"))
                    .set_description(t("settings.system.ytdlpRemoveConfirm"))
                    .set_buttons(rfd::MessageButtons::YesNo)
                    .show() == rfd::MessageDialogResult::Yes;
                if accepted {
                    if let Err(error) = crate::video::optional_ytdlp::remove(&self.workspace.data_dir) {
                        report!("yt-dlp opt-in remove refused: {error}");
                    }
                }
                Effect::Content
            }
            Action::RefreshYtDlp => Effect::Content,
            _ => Effect::None,
        }
    }
}
