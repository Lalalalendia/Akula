//! VIDEO-20: choose the downloader without ever turning the legacy signed
//! three-executable bundle HOLD into shipping authority.
//!
//! The installed Windows queue has already obtained the exact SHA-pinned
pub(super) fn download_one(
    library: &super::library::Library,
    executable: &std::ffi::OsStr,
    quickjs: Option<&std::ffi::OsStr>, url: &str,
    on_progress: impl FnMut(super::download::Progress),
) -> Result<super::download::Completed, String> {
    #[cfg(all(windows, feature = "installed", not(feature = "video-ui-e2e")))]
    {
        let runtime = quickjs.ok_or("Install verified QuickJS in Settings > System")?;
        super::download::download_one_with_js(library, executable, runtime, url, on_progress)
    }
    #[cfg(not(all(windows, feature = "installed", not(feature = "video-ui-e2e"))))]
    {
        let _ = quickjs;
        super::light::download_one_or_legacy(library, executable, url, on_progress)
    }
}
