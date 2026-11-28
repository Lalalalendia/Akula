//! Inert B1b0 staging uses the shared, tested B1a signed-release identity.
//! Core re-exports this implementation through its existing opt-in Linux API.
#[cfg(target_os = "linux")]
pub use akula_server_release_b1a as server_release_lab;
#[cfg(target_os = "linux")]
pub mod server_staging_lab;
