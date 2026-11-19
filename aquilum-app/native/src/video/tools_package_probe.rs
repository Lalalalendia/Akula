//! VIDEO-TOOLS-01e: native read-only status of the optional Windows package.
//! Never exposes executable paths or authorizes tool launch.
use super::tools_package::{
    APPROVED_VIDEO_TOOLS_KEY, MAX_MANIFEST, MAX_SIGNATURE,
    bounded_metadata, shipping_admission_enabled, verify_signed,
};
#[path = "tools_package_windows.rs"]
pub(super) mod windows_discovery;
/// Native Video Tools discovery is diagnostics only. Not executable admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolsDiscovery {
    Missing,
    Unsafe,
    PublisherNotApproved,
    IncompatibleVersion,
    ReadOnlyAttested,
}
/// This is only a *classification* of an already authenticated manifest.
/// It takes no public key, path or environment input and grants no execution.
/// Never use this as a substitute for signature/ACL/held-handle admission.
pub(crate) fn classify_authenticated_version(version: &str) -> ToolsDiscovery {
    match version {
        "0.1.0" => ToolsDiscovery::ReadOnlyAttested,
        _ => ToolsDiscovery::IncompatibleVersion,
    }
}
/// Read-only diagnostic state, never an executable path or launch permit.
pub(crate) fn probe_installed_video_tools() -> ToolsDiscovery {
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
    // Version selection follows publisher-authenticated contents only.
    match verify_signed(&root, &raw, signed, APPROVED_VIDEO_TOOLS_KEY) {
        Ok(attested) => classify_authenticated_version(&attested.version),
        Err(_) => ToolsDiscovery::Unsafe,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_version_classification_is_read_only_and_exact() {
        assert_eq!(classify_authenticated_version("0.1.0"), ToolsDiscovery::ReadOnlyAttested);
        for version in ["0.0.9", "0.1.1", "0.2.0", "1.0.0", "0.01.0", ""] {
            assert_eq!(classify_authenticated_version(version), ToolsDiscovery::IncompatibleVersion);
        }
    }
    #[test]
    fn native_discovery_cannot_approve_an_unknown_publisher() {
        assert_ne!(probe_installed_video_tools(), ToolsDiscovery::ReadOnlyAttested);
        assert!(!shipping_admission_enabled());
        assert!(APPROVED_VIDEO_TOOLS_KEY.is_empty());
    }
}
