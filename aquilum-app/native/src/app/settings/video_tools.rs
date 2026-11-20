use super::*;
use crate::i18n::t;
fn approved(title: &str, description: &str) -> bool {
    rfd::MessageDialog::new().set_title(t(title)).set_description(t(description))
        .set_buttons(rfd::MessageButtons::YesNo).show() == rfd::MessageDialogResult::Yes
}
impl App {
    pub(super) fn press_video_tool(&mut self, action: Action, section: Section) -> Effect {
        if section != Section::System { return Effect::None; }
        match action {
            Action::InstallYtDlp => {
                if approved("settings.system.ytdlpConfirmTitle", "settings.system.ytdlpConfirmText") {
                    let wake = std::sync::Arc::clone(&self.workspace.wake);
                    if let Err(e) = crate::video::optional_ytdlp::start(self.workspace.data_dir.clone(), wake) { report!("yt-dlp opt-in refused: {e}"); }
                }
                Effect::Content
            }
            Action::RemoveYtDlp => {
                if approved("settings.system.ytdlpRemoveTitle", "settings.system.ytdlpRemoveConfirm") {
                    if let Err(e) = crate::video::optional_ytdlp::remove(&self.workspace.data_dir) { report!("yt-dlp removal refused: {e}"); }
                }
                Effect::Content
            }
            Action::InstallQuickJs => {
                if approved("settings.system.quickjsConfirmTitle", "settings.system.quickjsConfirmText") {
                    let wake = std::sync::Arc::clone(&self.workspace.wake);
                    if let Err(e) = crate::video::optional_ytdlp::quickjs::start(self.workspace.data_dir.clone(), wake) { report!("QuickJS opt-in refused: {e}"); }
                }
                Effect::Content
            }
            Action::RemoveQuickJs => {
                if approved("settings.system.quickjsRemoveTitle", "settings.system.quickjsRemoveConfirm") {
                    if let Err(e) = crate::video::optional_ytdlp::quickjs::remove(&self.workspace.data_dir) { report!("QuickJS removal refused: {e}"); }
                }
                Effect::Content
            }
            Action::RefreshYtDlp | Action::RefreshQuickJs => Effect::Content,
            _ => Effect::None,
        }
    }
}
