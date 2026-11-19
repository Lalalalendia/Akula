//! VIDEO-TOOLS-01e: native read-only status of the optional Windows package.
//! Never exposes executable paths or authorizes tool launch.
use super::tools_package::{
    APPROVED_VIDEO_TOOLS_KEY, MAX_MANIFEST, MAX_SIGNATURE,
    bounded_metadata, shipping_admission_enabled, verify_signed,
};

#[path = "tools_package_windows.rs"]
mod windows_discovery;

/// Native Video Tools discovery is diagnostics only. Not executable admission.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ToolsDiscovery {
    Missing,
    Unsafe,
    PublisherNotApproved,
    ReadOnlyAttested,
}

pub(super) fn probe_installed_video_tools() -> ToolsDiscovery {
    let root = match windows_discovery::installed_root() {
        Ok(Some(root)) => root,
        Ok(None) => return ToolsDiscovery::Missing,
        Err(_) => return ToolsDiscovery::Unsafe,
    };
    // Exact publisher product contents: no environment-selected EXE fallback.
    let files = [
        "manifest.json",
        "manifest.minisig",
        "akula-video-mux.exe",
        "akula-yt-dlp.exe",
        "akula-quickjs.exe",
        "uninstall.exe",
    ];
    if windows_discovery::check_installed_acl(&root, &files).is_err() {
        return ToolsDiscovery::Unsafe;
    }
    // An ephemeral CI Minisign key must never enable production discovery.
    if APPROVED_VIDEO_TOOLS_KEY.is_empty() || !shipping_admission_enabled() {
        return ToolsDiscovery::PublisherNotApproved;
    }
    let raw = match bounded_metadata(&root.join("manifest.json"), MAX_MANIFEST) {
        Ok(raw) => raw,
        Err(_) => return ToolsDiscovery::Unsafe,
    };
    let signature = match bounded_metadata(&root.join("manifest.minisig"), MAX_SIGNATURE) {
        Ok(bytes) => bytes,
        Err(_) => return ToolsDiscovery::Unsafe,
    };
    let signed = match std::str::from_utf8(&signature) {
        Ok(signed) => signed,
        Err(_) => return ToolsDiscovery::Unsafe,
    };
    if verify_signed(&root, &raw, signed, APPROVED_VIDEO_TOOLS_KEY).is_err() {
        return ToolsDiscovery::Unsafe;
    }
    ToolsDiscovery::ReadOnlyAttested
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_discovery_cannot_approve_an_unknown_publisher() {
        assert_ne!(probe_installed_video_tools(), ToolsDiscovery::ReadOnlyAttested);
        assert!(!shipping_admission_enabled());
        assert!(APPROVED_VIDEO_TOOLS_KEY.is_empty());
    }
}
