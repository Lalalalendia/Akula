//! Inert, runtime-independent public plugin-package protocol (PLUGIN-01).
//!
//! Parsing a manifest never installs, loads, runs, or grants authority to a plugin.
//! Publisher claims and requested permissions remain untrusted data until a
//! future host-authenticated installation and permission broker exists.

mod manifest;

pub use manifest::{
    ApiRequirement, Capability, HostPlatform, ManifestError, PermissionRequests, PluginDependency,
    PluginManifest, PluginRuntime, CURRENT_API_LEVEL, CURRENT_API_NAME,
};

#[cfg(test)]
mod tests;
