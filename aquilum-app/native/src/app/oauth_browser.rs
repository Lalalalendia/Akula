//! Launch only the provider-generated OAuth authorization URL; never log its state.
use std::process::{Command, Stdio};

pub(super) fn open_browser(url: &str) -> bool {
    let binary = if cfg!(target_os = "windows") {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // Argument is passed directly, without shell interpretation. OAuth URL
    // and children stdout/stderr are never included in application logs.
    Command::new(binary).arg(url)
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().is_ok()
}
