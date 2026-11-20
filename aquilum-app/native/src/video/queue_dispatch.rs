//! VIDEO-20: choose the downloader without ever turning the legacy signed
//! three-executable bundle HOLD into shipping authority.
//!
//! The installed Windows queue has already obtained the exact SHA-pinned
//! optional executable from Settings; use the single-file legacy adapter,
//! not the experimental 3-EXE mux path (which remains on release HOLD).
pub(super) fn download_one(
    library: &super::library::Library,
    executable: &std::ffi::OsStr,
    url: &str,
    on_progress: impl FnMut(super::download::Progress),
) -> Result<std::path::PathBuf, String> {
    #[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
    {
        super::download::download_one(library, executable, url, on_progress)
    }
    #[cfg(not(all(windows, feature = "installed", not(feature = "video-ui-e2e"))))]
    {
        super::light::download_one_or_legacy(library, executable, url, on_progress)
    }
}
