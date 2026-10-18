//! SYNC-05a: S3 object-key namespace and immutable-manifest pointer contract.
//!
//! No network client, credentials, writes or scheduler are enabled here.
//! S3's ordinary paginated object listing is NOT a point-in-time snapshot.
//! A future S3 RemoteStore may report snapshot_consistency only after it
//! implements a verified immutable manifest + conditional root-pointer protocol.

use serde::{Deserialize, Serialize};

use super::{OperationId, RevisionId, VaultId};

const ROOT_SCHEMA: u16 = 1;
const MAX_ROOT_BYTES: usize = 4096;
const MAX_PREFIX_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum S3ContractError {
    InvalidPrefix,
    InvalidContentHash,
    InvalidRoot,
    ForeignVault,
    UnsupportedRootSchema,
    OversizedRoot,
}

/// One self-contained, vault-scoped namespace inside a user-selected bucket.
/// A cloud key NEVER contains a local pathname, filename or editor secret.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3Keyspace {
    prefix: String,
    vault_id: VaultId,
}

impl S3Keyspace {
    /// Restrict prefixes to uncomplicated, deterministic ASCII components.
    /// Provider bucket names, endpoints and credentials belong to the S3
    /// profile and are intentionally absent from this remote key contract.
    pub fn new(prefix: &str, vault_id: VaultId) -> Result<Self, S3ContractError> {
        let valid = !prefix.is_empty()
            && prefix.len() <= MAX_PREFIX_BYTES
            && prefix.split('/').all(|part| {
                !part.is_empty()
                    && part.len() <= 63
                    && part.bytes().all(|ch| {
                        ch.is_ascii_lowercase()
                            || ch.is_ascii_digit()
                            || ch == b'-'
                            || ch == b'_'
                    })
            });
        if !valid {
            return Err(S3ContractError::InvalidPrefix);
        }
        Ok(Self { prefix: prefix.to_owned(), vault_id })
    }

    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    fn vault_root(&self) -> String {
        format!("{}/v1/vaults/{}", self.prefix, self.vault_id)
    }

    /// Read-only S3 listing prefix; a raw ListObjectsV2 page is never a
    /// stable snapshot or an authorization to remove local data.
    pub(crate) fn vault_prefix(&self) -> String {
        format!("{}/", self.vault_root())
    }

    /// Reject arbitrary remote object keys before any SDK call, even for
    /// diagnostics. Do not expose another vault/profile's cloud objects.
    pub(crate) fn contains_key(&self, key: &str) -> bool {
        if key == self.current_head_key() {
            return true;
        }
        let prefix = self.vault_prefix();
        let Some(rest) = key.strip_prefix(&prefix) else {
            return false;
        };
        if let Some(hash) = rest.strip_prefix("blobs/") {
            return valid_blake3(hash);
        }
        if let Some(manifest) = rest.strip_prefix("manifests/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            return manifest.parse::<OperationId>().is_ok();
        }
        // Immutable, deterministic advisory locator for one exact V1
        // generation. Never treat a missing locator as an empty delta.
        if let Some(manifest) = rest.strip_prefix("coord/letters/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            return manifest.parse::<OperationId>().is_ok();
        }
        if let Some(revision) = rest.strip_prefix("revisions/")
            .and_then(|s| s.strip_suffix(".json"))
        {
            return revision.parse::<RevisionId>().is_ok();
        }
        false
    }

    /// Mutated ONLY with a provider-enforced If-None-Match:* or If-Match guard.
    /// The root's own ETag is a CAS validator, NOT a content digest.
    pub fn current_head_key(&self) -> String {
        format!("{}/refs/current.json", self.vault_root())
    }

    pub fn immutable_manifest_key(&self, manifest: OperationId) -> String {
        format!("{}/manifests/{manifest}.json", self.vault_root())
    }

    pub fn immutable_revision_key(&self, revision: RevisionId) -> String {
        format!("{}/revisions/{revision}.json", self.vault_root())
    }

    /// Discover a compact read-only delta letter from a *verified* V1
    /// manifest identity without S3 LIST or caller-supplied digest guessing.
    /// This is a separate write-once hint key, not the canonical V1 root.
    pub fn immutable_coord_letter_locator_key(&self, manifest: OperationId) -> String {
        format!("{}/coord/letters/{manifest}.json", self.vault_root())
    }

    pub fn immutable_blob_key(&self, blake3_hex: &str) -> Result<String, S3ContractError> {
        if !valid_blake3(blake3_hex) {
            return Err(S3ContractError::InvalidContentHash);
        }
        Ok(format!("{}/blobs/{blake3_hex}", self.vault_root()))
    }
}

fn valid_blake3(hash: &str) -> bool {
    hash.len() == 64
        && hash.bytes().all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

/// Small mutable pointer to a *previously uploaded and verified* immutable
/// manifest. A successor must be published by the provider's atomic CAS, not
/// by a client-side HEAD/PUT sequence.
///
/// This structure contains no document paths, credentials or encryption keys.
/// Future incompatible schemas and extra authority fields must fail closed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S3HeadPointerV1 {
    schema: u16,
    vault_id: VaultId,
    generation: u64,
    manifest_id: OperationId,
    manifest_blake3: String,
    previous_manifest_id: Option<OperationId>,
}

impl S3HeadPointerV1 {
    pub fn new(
        keyspace: &S3Keyspace,
        generation: u64,
        manifest_id: OperationId,
        manifest_blake3: &str,
        previous_manifest_id: Option<OperationId>,
    ) -> Result<Self, S3ContractError> {
        let head = Self {
            schema: ROOT_SCHEMA,
            vault_id: keyspace.vault_id(),
            generation,
            manifest_id,
            manifest_blake3: manifest_blake3.to_owned(),
            previous_manifest_id,
        };
        head.validate(keyspace)?;
        Ok(head)
    }

    pub fn decode(bytes: &[u8], keyspace: &S3Keyspace) -> Result<Self, S3ContractError> {
        if bytes.len() > MAX_ROOT_BYTES {
            return Err(S3ContractError::OversizedRoot);
        }
        let head: Self = serde_json::from_slice(bytes).map_err(|_| S3ContractError::InvalidRoot)?;
        head.validate(keyspace)?;
        Ok(head)
    }

    pub fn encode(&self, keyspace: &S3Keyspace) -> Result<Vec<u8>, S3ContractError> {
        self.validate(keyspace)?;
        serde_json::to_vec(self).map_err(|_| S3ContractError::InvalidRoot)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn manifest_id(&self) -> OperationId {
        self.manifest_id
    }

    pub fn manifest_blake3(&self) -> &str {
        &self.manifest_blake3
    }

    pub fn previous_manifest_id(&self) -> Option<OperationId> {
        self.previous_manifest_id
    }

    pub fn immutable_manifest_key(&self, keyspace: &S3Keyspace) -> Result<String, S3ContractError> {
        self.validate(keyspace)?;
        Ok(keyspace.immutable_manifest_key(self.manifest_id))
    }

    /// Direct-parent relation only. Multi-generation or two-client recovery
    /// must independently read and verify each immutable ancestor.
    pub fn is_direct_successor_of(&self, parent: &Self) -> bool {
        self.vault_id == parent.vault_id
            && parent.generation.checked_add(1) == Some(self.generation)
            && self.previous_manifest_id == Some(parent.manifest_id)
            && self.manifest_id != parent.manifest_id
    }

    fn validate(&self, keyspace: &S3Keyspace) -> Result<(), S3ContractError> {
        if self.schema != ROOT_SCHEMA {
            return Err(S3ContractError::UnsupportedRootSchema);
        }
        if self.vault_id != keyspace.vault_id {
            return Err(S3ContractError::ForeignVault);
        }
        if !valid_blake3(&self.manifest_blake3)
            || self.generation == 0
            || (self.generation == 1) != self.previous_manifest_id.is_none()
            || self.previous_manifest_id == Some(self.manifest_id)
        {
            return Err(S3ContractError::InvalidRoot);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> S3Keyspace {
        S3Keyspace::new("akula-sync/desktop", VaultId::new()).unwrap()
    }

    fn digest(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    #[test]
    fn remote_keys_are_deterministic_vault_scoped_and_never_expose_note_paths() {
        let ns = layout();
        let blob = digest(b"book markdown content");
        let manifest = OperationId::new();
        let revision = RevisionId::new();
        let root = format!("akula-sync/desktop/v1/vaults/{}", ns.vault_id());
        assert_eq!(ns.current_head_key(), format!("{root}/refs/current.json"));
        assert_eq!(ns.immutable_manifest_key(manifest), format!("{root}/manifests/{manifest}.json"));
        assert_eq!(ns.immutable_revision_key(revision), format!("{root}/revisions/{revision}.json"));
        assert_eq!(ns.immutable_blob_key(&blob).unwrap(), format!("{root}/blobs/{blob}"));
        for key in [
            ns.current_head_key(),
            ns.immutable_manifest_key(manifest),
            ns.immutable_revision_key(revision),
            ns.immutable_blob_key(&blob).unwrap(),
        ] {
            assert!(!key.contains(".md"));
            assert!(!key.contains("notes/"));
            assert!(!key.contains("chapter"));
        }
    }

    #[test]
    fn refuses_ambiguous_remote_prefixes_and_noncanonical_hashes() {
        let id = VaultId::new();
        for prefix in [
            "", "/root", "root/", "root//next", ".", "..", "akula/../evil",
            "akula\\evil", "akula%2Fescape", "UPPER", "café", "a b", "a\nb",
        ] {
            assert_eq!(S3Keyspace::new(prefix, id), Err(S3ContractError::InvalidPrefix), "{prefix:?}");
        }
        let huge = "a".repeat(MAX_PREFIX_BYTES + 1);
        assert_eq!(S3Keyspace::new(&huge, id), Err(S3ContractError::InvalidPrefix));
        let ns = layout();
        for invalid in ["", "0", &"g".repeat(64), &"A".repeat(64), &"0".repeat(65)] {
            assert_eq!(ns.immutable_blob_key(invalid), Err(S3ContractError::InvalidContentHash));
        }
    }

    #[test]
    fn sdk_namespace_gate_rejects_foreign_vaults_and_malformed_keys() {
        let ns = layout();
        let valid_blob = ns.immutable_blob_key(&digest(b"body")).unwrap();
        assert!(ns.contains_key(&valid_blob));
        assert!(ns.contains_key(&ns.current_head_key()));
        assert!(ns.contains_key(&ns.immutable_manifest_key(OperationId::new())));
        assert!(ns.contains_key(&ns.immutable_revision_key(RevisionId::new())));
        for bad in [
            "akula-sync/desktop/v1/vaults/other/refs/current.json".to_owned(),
            format!("{}/blobs/../another-vault", ns.vault_prefix()),
            format!("{}manifests/../other.json", ns.vault_prefix()),
            format!("{}blobs/not-a-hash", ns.vault_prefix()),
            format!("{}credentials.env", ns.vault_prefix()),
            format!("{}revisions/.json", ns.vault_prefix()),
        ] {
            assert!(!ns.contains_key(&bad), "must refuse {bad}");
        }
    }

    #[test]
    fn immutable_pointer_roundtrip_and_strict_forward_schema_rejection() {
        let ns = layout();
        let id = OperationId::new();
        let first = S3HeadPointerV1::new(&ns, 1, id, &digest(b"manifest v1"), None).unwrap();
        let json = first.encode(&ns).unwrap();
        assert_eq!(S3HeadPointerV1::decode(&json, &ns), Ok(first.clone()));
        assert_eq!(first.immutable_manifest_key(&ns).unwrap(), ns.immutable_manifest_key(id));
        let mut edited: serde_json::Value = serde_json::from_slice(&json).unwrap();
        edited["schema"] = 2.into();
        assert_eq!(
            S3HeadPointerV1::decode(&serde_json::to_vec(&edited).unwrap(), &ns),
            Err(S3ContractError::UnsupportedRootSchema),
        );
        edited["schema"] = 1.into();
        edited["extra_authority"] = true.into();
        assert_eq!(
            S3HeadPointerV1::decode(&serde_json::to_vec(&edited).unwrap(), &ns),
            Err(S3ContractError::InvalidRoot),
        );
    }

    #[test]
    fn pointer_rejects_other_vault_corruption_rollback_and_unbounded_input() {
        let ns = layout();
        let other = layout();
        let initial = S3HeadPointerV1::new(
            &ns, 1, OperationId::new(), &digest(b"manifest"), None,
        ).unwrap();
        assert_eq!(
            S3HeadPointerV1::decode(&initial.encode(&ns).unwrap(), &other),
            Err(S3ContractError::ForeignVault),
        );
        let mut edited: serde_json::Value = serde_json::from_slice(&initial.encode(&ns).unwrap()).unwrap();
        edited["manifest_blake3"] = "bad".into();
        assert_eq!(
            S3HeadPointerV1::decode(&serde_json::to_vec(&edited).unwrap(), &ns),
            Err(S3ContractError::InvalidRoot),
        );
        edited["manifest_blake3"] = digest(b"fixed").into();
        edited["generation"] = 0.into();
        assert_eq!(
            S3HeadPointerV1::decode(&serde_json::to_vec(&edited).unwrap(), &ns),
            Err(S3ContractError::InvalidRoot),
        );
        assert_eq!(
            S3HeadPointerV1::decode(&vec![b'x'; MAX_ROOT_BYTES + 1], &ns),
            Err(S3ContractError::OversizedRoot),
        );
    }

    #[test]
    fn direct_successor_requires_strict_generation_and_verified_parent() {
        let ns = layout();
        let parent_id = OperationId::new();
        let parent = S3HeadPointerV1::new(&ns, 1, parent_id, &digest(b"parent"), None).unwrap();
        let child = S3HeadPointerV1::new(
            &ns, 2, OperationId::new(), &digest(b"child"), Some(parent_id),
        ).unwrap();
        assert!(child.is_direct_successor_of(&parent));
        assert!(!parent.is_direct_successor_of(&child));
        assert!(!S3HeadPointerV1::new(
            &ns, 3, OperationId::new(), &digest(b"skipped"), Some(parent_id),
        ).unwrap().is_direct_successor_of(&parent));
        assert_eq!(
            S3HeadPointerV1::new(&ns, 2, OperationId::new(), &digest(b"missing parent"), None),
            Err(S3ContractError::InvalidRoot),
        );
        assert_eq!(
            S3HeadPointerV1::new(&ns, 2, parent_id, &digest(b"reuse id"), Some(parent_id)),
            Err(S3ContractError::InvalidRoot),
        );
    }
}
