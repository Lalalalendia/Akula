//! VIDEO-TOOLS-01m: a diagnostic label for already-authenticated metadata.
//! This module neither verifies a caller-selected key nor authorizes execution.
//! Call only after the signed package, OS-known root and protected ACL are verified.
use super::ToolsDiscovery;

/// The standalone test witness uses the same post-signature version label
/// as the installed product; no ephemeral publisher key enters that product.
pub(crate) fn classify_authenticated_version(version: &str) -> ToolsDiscovery {
    match version {
        "0.1.0" => ToolsDiscovery::ReadOnlyAttested,
        _ => ToolsDiscovery::IncompatibleVersion,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_version_classification_is_exact() {
        assert_eq!(
            classify_authenticated_version("0.1.0"),
            ToolsDiscovery::ReadOnlyAttested
        );
        for version in ["0.0.9", "0.1.1", "0.2.0", "1.0.0", "0.01.0", ""] {
            assert_eq!(
                classify_authenticated_version(version),
                ToolsDiscovery::IncompatibleVersion
            );
        }
    }
}
