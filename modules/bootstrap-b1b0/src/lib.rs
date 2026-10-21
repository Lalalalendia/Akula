//! Inert B1b0 staging uses the shared, tested B1a signed-release identity.
//! Production Core remains unchanged until its own compatibility gate passes.
#[cfg(target_os = "linux")]
pub use akula_server_release_b1a as server_release_lab;
#[cfg(target_os = "linux")]
pub mod server_staging_lab;
