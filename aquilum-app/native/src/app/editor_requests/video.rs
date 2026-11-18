//! User-facing admission of an external yt-dlp in the two-EXE stock installer.
//! This is a presence hint, not a signature or process trust verification.
use std::sync::Arc;
use masonry::app::RenderRoot;
use super::App;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use super::Popup;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use masonry::kurbo::Point;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use crate::i18n::t;
#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
use crate::ui::dialog::ConfirmDialog;

#[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
fn stock_external_ytdlp_available(
    selected: Option<&std::ffi::OsStr>, search_path: Option<&std::ffi::OsStr>,
) -> bool {
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;
    // Never treat a directory, symlink/reparse file or relative override as
    // installed external tooling. This check only avoids a guaranteed ENOENT.
    let regular = |path: &Path| {
        path.is_absolute() && std::fs::symlink_metadata(path).is_ok_and(|meta| {
            meta.is_file() && !meta.file_type().is_symlink()
                && meta.file_attributes() & 0x400 == 0
        })
    };
    if let Some(value) = selected.filter(|v| !v.is_empty()) {
        return regular(Path::new(value));
    }
    search_path.into_iter().flat_map(std::env::split_paths)
        .filter(|dir| dir.is_absolute())
        .any(|dir| regular(&dir.join("yt-dlp.exe")))
}

impl App {
    pub(super) fn download_video(&mut self, root: &mut RenderRoot, url: &str) {
        #[cfg(all(windows, feature = "video-ui-e2e"))]
        crate::video::ui_acceptance::mark("CLICK_DOWNLOAD");
        // The stock signed installer intentionally contains only Akula and
        // Shiva. Do not enqueue a background job that cannot possibly start.
        // The non-shipping five-EXE VIDEO-18 lab remains unaffected.
        #[cfg(all(windows, feature = "installed", not(feature = "video-rust-bundle")))]
        if !stock_external_ytdlp_available(
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
            // The existing dismiss-only notice slot handles either button.
            self.popup = Some(Popup::PdfNotice { dialog });
            return;
        }
        #[cfg(not(all(windows, feature = "installed", not(feature = "video-rust-bundle"))))]
        let _ = root;
        let embeds = self.embeds.clone();
        let wake = Arc::clone(&self.workspace.wake);
        let changed: Arc<dyn Fn(bool, &str) + Send + Sync> = Arc::new(move |finished, source| {
            if let Some(embeds) = &embeds {
                embeds.video_update(source, finished);
            } else {
                wake();
            }
            if finished && let Some(crate::video::queue::Status::Failed(reason)) = crate::video::queue::status(source) {
                report!("локальное видео не загружено: {reason}");
            }
        });
        if let Err(reason) = crate::video::queue::enqueue(
            url.to_owned(), self.workspace.data_dir.clone(),
            self.workspace.core.known_roots(), changed,
        ) {
            report!("видеозагрузка отклонена: {reason}");
        }
    }
}

#[cfg(all(test, windows, feature = "installed", not(feature = "video-rust-bundle")))]
mod tests {
    use super::stock_external_ytdlp_available;
    #[test]
    fn stock_download_missing_dependency_never_admits_a_job() {
        assert!(!stock_external_ytdlp_available(None, None));
        let sandbox = tempfile::tempdir().unwrap();
        let binary = sandbox.path().join("yt-dlp.exe");
        std::fs::write(&binary, b"MZ not executed").unwrap();
        assert!(stock_external_ytdlp_available(Some(binary.as_os_str()), None));
        let paths = std::env::join_paths([sandbox.path()]).unwrap();
        assert!(stock_external_ytdlp_available(None, Some(paths.as_os_str())));
        assert!(!stock_external_ytdlp_available(
            Some(sandbox.path().join("absent.exe").as_os_str()), Some(paths.as_os_str())
        ));
        std::fs::remove_file(binary).unwrap();
        assert!(!stock_external_ytdlp_available(None, Some(paths.as_os_str())));
    }
}
