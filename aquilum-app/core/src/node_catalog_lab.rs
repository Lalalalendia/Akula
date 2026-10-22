//! Compatibility adapter preserving the Core VaultId custody boundary.
//! The separate Node Catalog crate stores equivalent typed UUIDs, but cannot
//! authenticate a vault. Original operator admission remains in Core/host.
pub use akula_node_catalog::{CatalogError, ListedDocument, ReadDocument,
    MAX_DOCUMENT_BYTES, MAX_DOCUMENTS};
use crate::sync::VaultId;
use std::fs::File;
use std::path::Path;

pub struct ReadCatalog(akula_node_catalog::ReadCatalog);
impl ReadCatalog {
    pub fn open(root: &Path, id: VaultId, allowed: &[&str]) -> Result<Self,CatalogError> {
        akula_node_catalog::ReadCatalog::open(root,
            akula_node_catalog::VaultId::from_uuid(id.as_uuid()),allowed).map(Self)
    }
    pub fn from_pinned_root(root: File, id: VaultId, allowed: &[&str])
        -> Result<Self,CatalogError> {
        akula_node_catalog::ReadCatalog::from_pinned_root(root,
            akula_node_catalog::VaultId::from_uuid(id.as_uuid()),allowed).map(Self)
    }
    pub fn vault_id(&self) -> VaultId {
        VaultId::from_uuid(self.0.vault_id().as_uuid())
    }
    pub fn list(&self) -> Vec<ListedDocument> { self.0.list() }
    pub fn read(&self,id:&str) -> Result<ReadDocument,CatalogError> {self.0.read(id)}
    pub fn read_if_revision(&self,id:&str,expected:Option<&str>)
        -> Result<ReadDocument,CatalogError> {self.0.read_if_revision(id,expected)}
}
