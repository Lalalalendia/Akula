//! Platform-specific first actually presented GUI-frame acknowledgements.
//! Windows VIDEO-18c remains test-only and cannot confirm an update. Linux and
//! macOS preserve their existing installed-mode behavior without changing the
//! normal publisher/admission or update engine flow.
#[cfg(all(target_os = "windows", feature = "installed",
    feature = "video-rust-bundle", feature = "video-ui-e2e"))]
#[path = "windows_first_gui_ack.rs"]
mod windows_first_gui_ack;

#[cfg(all(target_os = "linux", feature = "installed"))]
pub(super) fn on_first_presented_frame() {
    super::linux_first_gui_ack::on_first_presented_frame();
}

#[cfg(all(target_os = "macos", feature = "installed"))]
pub(super) fn on_first_presented_frame() {
    super::startup_ack::on_first_presented_frame();
}

#[cfg(all(target_os = "windows", feature = "installed",
    feature = "video-rust-bundle", feature = "video-ui-e2e"))]
pub(super) fn on_first_presented_frame() {
    windows_first_gui_ack::on_first_presented_frame();
}

#[cfg(not(any(
    all(target_os = "linux", feature = "installed"),
    all(target_os = "macos", feature = "installed"),
    all(target_os = "windows", feature = "installed",
        feature = "video-rust-bundle", feature = "video-ui-e2e")
)))]
#[allow(dead_code)]
pub(super) fn on_first_presented_frame() {
    // Ordinary Windows two-EXE and unsupported platforms never emit the lab
    // receipt. They cannot accidentally authorize the video generation.
}
