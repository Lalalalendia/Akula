//! VIDEO-20: installed yt-dlp is user-installed and hash-checked; old bundle stays HOLD.
use super::App;
use masonry::app::RenderRoot;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))] use super::Popup;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))] use crate::i18n::t;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))] use crate::ui::dialog::ConfirmDialog;
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))] use crate::video::optional_ytdlp::{self, Status};
#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))] use masonry::kurbo::Point;

#[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
fn notice_keys(status: &Status) -> (&'static str, &'static str) {
    match status {
        Status::Missing => ("editor.videoToolsMissingTitle", "editor.videoToolsMissing"),
        Status::Downloading(_) => ("editor.videoToolsDownloadingTitle", "editor.videoToolsDownloading"),
        Status::Invalid => ("editor.videoToolsUnsafeTitle", "editor.videoToolsUnsafe"),
        Status::Failed(_) => ("editor.videoToolsDownloadFailedTitle", "editor.videoToolsDownloadFailed"),
        Status::Installed => ("editor.videoToolsAttestedTitle", "editor.videoToolsAttested"),
    }
}

impl App {
    pub(super) fn optional_video_tools_notice(&mut self, root: &mut RenderRoot) -> bool {
        #[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
        {
            // A real, fixed, SHA-256-verified local optional file can proceed.
            // PATH, env-selected executables, old package signatures and
            // CI-only bypasses never authorize an installed download.
            let state = optional_ytdlp::status(&self.workspace.data_dir);
            if matches!(state, Status::Installed) {
                return false;
            }
            let (title, body) = notice_keys(&state);
            let (widget, dialog) = ConfirmDialog::new(
                &t(title), &t(body),
                &t("editor.table.studyClose"),
                &t("editor.table.studyOk"),
            );
            root.add_layer(widget, Point::ORIGIN);
            root.focus_on(Some(dialog.layer));
            self.popup = Some(Popup::PdfNotice { dialog });
            return true;
        }
        #[cfg(not(all(windows, feature = "installed", not(feature = "video-ui-e2e"))))]
        let _ = root;
        false
    }
}

#[cfg(all(test, windows, feature = "installed", not(feature = "video-ui-e2e")))]
mod tests {
    use super::*;

    #[test]
    fn optional_download_states_have_real_localized_messages() {
        let states = [
            Status::Missing, Status::Downloading(1), Status::Invalid,
            Status::Failed("network".into()), Status::Installed,
        ];
        let mut distinct = std::collections::HashSet::new();
        for state in &states {
            let (title, body) = notice_keys(state);
            assert!(distinct.insert((title, body)));
            for raw in [
                include_str!("../../../assets/locales/ru.json"),
                include_str!("../../../assets/locales/en.json"),
            ] {
                let locale: serde_json::Value = serde_json::from_str(raw).unwrap();
                for key in [title, body] {
                    let leaf = key.strip_prefix("editor.").unwrap();
                    assert!(!locale["editor"][leaf].as_str().unwrap_or("").is_empty(),
                        "missing video tool translation: {key}");
                }
            }
        }
    }
}
