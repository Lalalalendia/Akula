//! Isolated Linux-only D1.5 read-catalog parity experiment.
//! The original production Core catalog remains authoritative until
//! feature-gated compatibility, security and required CI tests pass.
//! This crate has no listener, File Gate writer, installer or network authority.
#[cfg(target_os = "linux")]
pub mod catalog;
#[cfg(target_os = "linux")]
pub use catalog::{CatalogError, ListedDocument, ReadCatalog, ReadDocument,
    MAX_DOCUMENTS, MAX_DOCUMENT_BYTES};
pub mod identity;
pub use identity::VaultId;

// The unmodified catalog.rs source imports crate::sync::VaultId.
// Keep this temporary compatibility namespace local to the isolated lab.
pub mod sync { pub use crate::identity::VaultId; }
