//! Experimental, Linux-only D1.5 read catalog. Not a network listener.
//!
//! A trusted operator pins one directory and an explicit list of Markdown
//! filenames. A later (separately authenticated) host may address ONLY the
//! returned opaque IDs. IDs are stable identifiers, NOT secret capabilities.
//!
//! No File Gate writes, MCP dispatch, search workers or directory traversal.
//! Fail closed on renamed/replaced file identity. The Linux root dirfd remains
//! bound to its original directory even after its external pathname changes.

use crate::sync::VaultId;
use rustix::fs::{open, openat, Mode, OFlags};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

pub const MAX_DOCUMENTS: usize = 16;
pub const MAX_DOCUMENT_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    InvalidConfiguration,
    UnknownObject,
    ReplacedObject,
    TooLarge,
    InvalidUtf8,
    RevisionConflict,
    Io,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedDocument {
    pub id: String,
    pub filename: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadDocument {
    pub id: String,
    pub content: String,
    /// BLAKE3 over original *raw bytes*, including CRLF if present.
    pub raw_revision: String,
}

struct Entry {
    filename: String,
    device: u64,
    inode: u64,
}

/// A trusted operator-defined, immutable map of file identities to IDs.
///
/// The list is static: adding documents or repointing an ID requires a new
/// explicit admission, not a URL query or a file system rescan.
pub struct ReadCatalog {
    vault_id: VaultId,
    root: File,
    entries: BTreeMap<String, Entry>,
}

impl ReadCatalog {
    /// Pin a trusted root and at most 16 top-level Markdown filenames.
    ///
    /// The caller must independently authorize this catalog for a particular
    /// user/device. Constructing one does NOT authenticate a network client.
    pub fn open(root: &Path, vault_id: VaultId, allowed: &[&str]) -> Result<Self, CatalogError> {
        if allowed.is_empty() || allowed.len() > MAX_DOCUMENTS {
            return Err(CatalogError::InvalidConfiguration);
        }
        let owned = open(
            root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ).map_err(|_| CatalogError::InvalidConfiguration)?;
        Self::from_pinned_root(File::from(owned), vault_id, allowed)
    }

    /// Admit from an OS directory handle that the host has ALREADY pinned
    /// and verified. D2 calls this with a duplicate of the D1 root fd: there
    /// is no second pathname resolution between the host's root preflight
    /// and catalog admission. The caller must separately verify that this
    /// fd belongs to an authorized VaultId; a typed UUID is not custody proof.
    pub fn from_pinned_root(
        root: File,
        vault_id: VaultId,
        allowed: &[&str],
    ) -> Result<Self, CatalogError> {
        if allowed.is_empty() || allowed.len() > MAX_DOCUMENTS {
            return Err(CatalogError::InvalidConfiguration);
        }
        if !root.metadata().map_err(|_| CatalogError::Io)?.is_dir() {
            return Err(CatalogError::InvalidConfiguration);
        }

        let mut entries = BTreeMap::new();
        for filename in allowed {
            validate_filename(filename)?;
            let id = object_id(vault_id, filename);
            if entries.contains_key(&id) {
                return Err(CatalogError::InvalidConfiguration);
            }
            let file = open_note(&root, filename)
                .map_err(|_| CatalogError::InvalidConfiguration)?;
            let metadata = file.metadata().map_err(|_| CatalogError::Io)?;
            if !metadata.is_file() {
                return Err(CatalogError::InvalidConfiguration);
            }
            if metadata.len() > MAX_DOCUMENT_BYTES {
                return Err(CatalogError::TooLarge);
            }
            entries.insert(id, Entry {
                filename: (*filename).to_owned(),
                device: metadata.dev(),
                inode: metadata.ino(),
            });
        }
        Ok(Self { vault_id, root, entries })
    }

    /// The caller must independently verify this explicit VaultId against its
    /// host authority. Never infer a shared VaultId from a local workspace path.
    pub fn vault_id(&self) -> VaultId { self.vault_id }

    /// Lists only the exact operator-admitted set. IDs are NOT credentials.
    pub fn list(&self) -> Vec<ListedDocument> {
        self.entries.iter().map(|(id, entry)| ListedDocument {
            id: id.clone(),
            filename: entry.filename.clone(),
        }).collect()
    }

    pub fn read(&self, object_id: &str) -> Result<ReadDocument, CatalogError> {
        self.read_if_revision(object_id, None)
    }

    /// Verify a raw-byte revision before returning content, or report conflict.
    /// No state is written and the content is never provided on conflict.
    pub fn read_if_revision(
        &self,
        id: &str,
        expected_revision: Option<&str>,
    ) -> Result<ReadDocument, CatalogError> {
        let entry = self.entries.get(id).ok_or(CatalogError::UnknownObject)?;
        let mut file = open_note(&self.root, &entry.filename)
            .map_err(|_| CatalogError::ReplacedObject)?;
        let before = file.metadata().map_err(|_| CatalogError::Io)?;
        if !before.is_file() || before.dev() != entry.device || before.ino() != entry.inode {
            return Err(CatalogError::ReplacedObject);
        }
        if before.len() > MAX_DOCUMENT_BYTES {
            return Err(CatalogError::TooLarge);
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        (&mut file).take(MAX_DOCUMENT_BYTES + 1)
            .read_to_end(&mut bytes).map_err(|_| CatalogError::Io)?;
        if bytes.len() as u64 > MAX_DOCUMENT_BYTES {
            return Err(CatalogError::TooLarge);
        }
        let after = file.metadata().map_err(|_| CatalogError::Io)?;
        if before.dev() != after.dev() || before.ino() != after.ino()
            || before.len() != after.len() || after.len() != bytes.len() as u64
            || before.mtime() != after.mtime() || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime() || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(CatalogError::ReplacedObject);
        }
        let raw_revision = blake3::hash(&bytes).to_hex().to_string();
        if expected_revision.is_some_and(|rev| rev != raw_revision) {
            return Err(CatalogError::RevisionConflict);
        }
        let content = String::from_utf8(bytes).map_err(|_| CatalogError::InvalidUtf8)?;
        Ok(ReadDocument { id: id.to_owned(), content, raw_revision })
    }
}

fn open_note(root: &File, filename: &str) -> Result<File, rustix::io::Errno> {
    openat(
        root,
        filename,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ).map(File::from)
}

fn validate_filename(filename: &str) -> Result<(), CatalogError> {
    if filename.is_empty() || filename.len() > 128
        || !filename.ends_with(".md")
        || filename.bytes().any(|byte| byte == b'\\' || byte.is_ascii_control())
    {
        return Err(CatalogError::InvalidConfiguration);
    }
    let mut parts = Path::new(filename).components();
    if !matches!(parts.next(), Some(Component::Normal(_))) || parts.next().is_some() {
        return Err(CatalogError::InvalidConfiguration);
    }
    Ok(())
}

fn object_id(vault_id: VaultId, filename: &str) -> String {
    // Stable across a restart/order change, but never a claim of authority.
    // Deliberately domain-separated from content revisions and peer identities.
    let mut hasher = blake3::Hasher::new_derive_key("Akula Node lab document ID v1");
    // Same filenames in different vaults MUST NOT alias. The VaultId comes
    // from an explicit trusted host registration, never from app-local paths.
    hasher.update(vault_id.as_uuid().as_bytes());
    hasher.update(filename.as_bytes());
    hasher.finalize().to_hex().to_string()
}
