//! Explicit, user-facing dependency notice for the stock two-EXE Windows installer.
//! A PATH presence hint is not signature verification or a launch authorization.
use super::App;
use masonry::app::RenderRoot;

#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use super::Popup;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use crate::i18n::t;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use crate::ui::dialog::ConfirmDialog;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use masonry::kurbo::Point;

#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
fn external_ytdlp_visible(
    selected: Option<&std::ffi::OsStr>,
    search_path: Option<&std::ffi::OsStr>,
) -> bool {
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;

    // A regular-looking file is not authenticated; this only avoids a
    // definitely unavailable downloader. No executable is run as a probe.
    let regular = |path: &Path| {
        path.is_absolute()
            && std::fs::symlink_metadata(path).is_ok_and(|meta| {
                meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.file_attributes() & 0x400 == 0
            })
    };
    if let Some(value) = selected.filter(|v| !v.is_empty()) {
        return regular(Path::new(value));
    }
    search_path
        .into_iter()
        .flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
        .any(|dir| regular(&dir.join("yt-dlp.exe")))
}

impl App {
    pub(super) fn stock_video_dependency_notice(&mut self, root: &mut RenderRoot) -> bool {
        #[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
        {
            if !external_ytdlp_visible(
                std::env::var_os("AKULA_YT_DLP_BIN").as_deref(),
                std::env::var_os("PATH").as_deref(),
            ) {
                let (widget, dialog) = ConfirmDialog::new(
                    &t("editor.videoDownloadUnavailableTitle"),
                    &t("editor.videoDownloadUnavailable"),
                    &t("editor.table.studyClose"),
                    &t("editor.table.studyOk"),
                );
                root.add_layer(widget, Point::ORIGIN);
                root.focus_on(Some(dialog.layer));
                // Existing dismiss-only modal handles both close actions.
                self.popup = Some(Popup::PdfNotice { dialog });
                return true;
            }
        }
        #[cfg(not(all(windows, feature = "installed", not(feature = "video-rust-bundle"))))]
        let _ = root;
        false
    }
}

#[cfg(all(test, windows, feature = "installed", not(feature = "video-rust-bundle")))]
mod tests {
    use super::external_ytdlp_visible;

    #[test]
    fn missing_stock_dependency_does_not_admit_a_background_download() {
        assert!(!external_ytdlp_visible(None, None));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yt-dlp.exe");
        std::fs::write(&path, b"MZ presence hint only").unwrap();

        assert!(external_ytdlp_visible(Some(path.as_os_str()), None));
        let search = std::env::join_paths([dir.path()]).unwrap();
        assert!(external_ytdlp_visible(None, Some(search.as_os_str())));
        assert!(!external_ytdlp_visible(
            Some(dir.path().join("absent.exe").as_os_str()),
            Some(search.as_os_str()),
        ));
        std::fs::remove_file(path).unwrap();
        assert!(!external_ytdlp_visible(None, Some(search.as_os_str())));
    }
}
