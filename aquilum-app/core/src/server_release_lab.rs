//! Offline B1a release verification; never authorizes installation.
pub use akula_server_release_b1a::*;
#[cfg(feature = "server-install-review")]
pub mod review;
