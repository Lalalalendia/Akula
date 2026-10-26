//! Compatibility export for existing Akula Core bootstrap-B0 callers.
//!
//! The implementation now lives in the isolated akula-server-bootstrap-b0
//! crate. This module preserves aquilum_core::server_bootstrap_lab::*.
//! No SSH or installer authority is introduced by this re-export.
pub use akula_server_bootstrap_b0::*;
