//! Bootstrap B1b0 parity package: inert Linux-only private staging.
//!
//! This experimental crate does not add remote installation or execution.
//! For now, its B1a dependency is the original Core source, not a second copy.
//! Production Core remains unchanged until a shared B1a crate exists.

#[cfg(target_os = "linux")]
#[path = "../../../aquilum-app/core/src/server_release_lab.rs"]
pub mod server_release_lab;

#[cfg(target_os = "linux")]
pub mod server_staging_lab;
