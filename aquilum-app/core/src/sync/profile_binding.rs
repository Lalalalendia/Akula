//! SYNC-05e-2: inert, app-private binding between one registered local root
//! and one explicitly selected shared S3 vault/profile.
//!
//! UI-state workspace UUIDs are machine-local and MUST NOT double as the
//! shared remote VaultId. This SQLite record keeps those identities separate.
//! It does not hold AWS credentials, authenticate a remote publisher, grant
//! mutation rights, bootstrap an absent S3 root, or advance a sync baseline.
//!
//! The host must independently fetch the verified S3 head, obtain user consent,
//! and bind an app-data database outside the workspace. Neither the digest
//! nor possession of this record is authorization to sync or write files.

use super::{
    RegisteredReadAdmission, S3ConnectionProfile, S3Keyspace,
    S3VerifiedHead, VaultId, VaultPortError,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug)]
pub(crate) enum S3BindingError {
    Admission(VaultPortError),
    Sqlite(rusqlite::Error),
    Io(std::io::Error),
    InvalidProfile,
    UnverifiedHead,
    BindingConflict,
    InvalidStoreLocation,
    CorruptedBinding,
    UnsupportedSchema(i64),
}

impl From<VaultPortError> for S3BindingError {
    fn from(error: VaultPortError) -> Self { Self::Admission(error) }
}
impl From<rusqlite::Error> for S3BindingError {
    fn from(error: rusqlite::Error) -> Self { Self::Sqlite(error) }
}
impl From<std::io::Error> for S3BindingError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

/// This is a durable *configuration record*, not a capability token.
/// A scoped S3 root may be shared by multiple independent app-data databases,
/// but a single app-data database never binds it to two different workspaces.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisteredS3Binding {
    pub workspace_id: Uuid,
    pub local_root_path: String,
    pub local_root_device: u64,
    pub local_root_file: u64,
    pub profile_id: String,
    pub vault_id: VaultId,
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub force_path_style: bool,
    pub remote_head_key: String,
}

impl RegisteredS3Binding {
    fn scope_hash(&self) -> Result<String, S3BindingError> {
        // The *remote namespace*, not a local registration UUID, identifies
        // one logical S3 vault. The fingerprint is for collision checks only.
        let bytes = serde_json::to_vec(&(
            &self.bucket, &self.region, &self.endpoint,
            self.force_path_style, &self.remote_head_key,
        )).map_err(|_| S3BindingError::CorruptedBinding)?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    pub(crate) fn matches_remote(
        &self,
        profile: &S3ConnectionProfile,
        keyspace: &S3Keyspace,
    ) -> bool {
        self.vault_id == keyspace.vault_id()
            && self.bucket == profile.bucket
            && self.region == profile.region
            && self.endpoint == profile.endpoint
            && self.force_path_style == profile.force_path_style
            && self.remote_head_key == keyspace.current_head_key()
    }
}

pub(crate) struct SqliteRegisteredS3Bindings {
    connection: Connection,
    database_path: PathBuf,
}

impl SqliteRegisteredS3Bindings {
    /// Refuse any app-data path inside the Core-registered workspace, including
    /// a symlinked directory. This is a *negative guard*, not an interprocess
    /// lease on the app-data pathname. No production write authority follows.
    pub(crate) fn open(
        admission: &RegisteredReadAdmission<'_>,
        database_path: &Path,
    ) -> Result<Self, S3BindingError> {
        admission.recheck_registered_root()?;
        validate_appdata_location(admission, database_path)?;
        let connection = Connection::open(database_path)?;
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        match version {
            0 => connection.execute_batch(
                "BEGIN IMMEDIATE;
                 CREATE TABLE registered_s3_bindings (
                     workspace_id TEXT PRIMARY KEY NOT NULL,
                     profile_id TEXT NOT NULL UNIQUE,
                     remote_scope_hash TEXT NOT NULL UNIQUE,
                     record_json BLOB NOT NULL,
                     record_hash TEXT NOT NULL
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?,
            SCHEMA_VERSION => {}
            other => return Err(S3BindingError::UnsupportedSchema(other)),
        };
        admission.recheck_registered_root()?;
        Ok(Self { connection, database_path: database_path.to_path_buf() })
    }

    /// A trusted host calls this only after separately checking the existing
    /// S3 head and obtaining user approval to join that exact remote namespace.
    /// Re-binding an established workspace or stealing another local
    /// workspace's remote scope fails without changing the existing record.
    ///
    /// The current SDK-level S3Head is a transport integrity proof; it is
    /// neither a publisher signature nor an anti-rollback guarantee.
    pub(crate) fn bind_existing_verified_s3(
        &mut self,
        admission: &RegisteredReadAdmission<'_>,
        profile_id: &str,
        profile: &S3ConnectionProfile,
        keyspace: &S3Keyspace,
        verified: &S3VerifiedHead,
    ) -> Result<RegisteredS3Binding, S3BindingError> {
        admission.recheck_registered_root()?;
        validate_appdata_location(admission, &self.database_path)?;
        if profile_id.is_empty()
            || profile_id.len() > 96
            || !profile_id.bytes().all(|b| b.is_ascii_lowercase()
                || b.is_ascii_digit() || b == b'-' || b == b'_')
            || profile.validate().is_err()
            || verified.etag.is_empty()
            || verified.etag.len() > 256
            || verified.etag.bytes().any(|b| b.is_ascii_control())
        {
            return Err(S3BindingError::InvalidProfile);
        }
        // Require a self-consistent *existing* remote pointer and manifest.
        // An absent root is not automatically an empty synchronized vault.
        if verified.pointer.encode(keyspace).is_err()
            || verified.manifest.encode(keyspace).is_err()
            || verified.manifest.manifest_id != verified.pointer.manifest_id()
            || verified.manifest.generation != verified.pointer.generation()
            || verified.manifest.previous_manifest_id
                != verified.pointer.previous_manifest_id()
            || verified.manifest.digest(keyspace).ok().as_deref()
                != Some(verified.pointer.manifest_blake3())
        {
            return Err(S3BindingError::UnverifiedHead);
        }

        let (device, file) = admission.registered_root_identity_for_profile()?;
        let proposal = RegisteredS3Binding {
            workspace_id: admission.registered_workspace_id(),
            local_root_path: admission.registered_root_path().to_str()
                .ok_or(S3BindingError::InvalidStoreLocation)?.to_owned(),
            local_root_device: device,
            local_root_file: file,
            profile_id: profile_id.to_owned(),
            vault_id: keyspace.vault_id(),
            bucket: profile.bucket.clone(),
            region: profile.region.clone(),
            endpoint: profile.endpoint.clone(),
            force_path_style: profile.force_path_style,
            remote_head_key: keyspace.current_head_key(),
        };
        let scope_hash = proposal.scope_hash()?;
        let record_json = serde_json::to_vec(&proposal)
            .map_err(|_| S3BindingError::CorruptedBinding)?;
        let digest = blake3::hash(&record_json).to_hex().to_string();

        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(previous) = load(&tx, proposal.workspace_id)? {
            if previous != proposal { return Err(S3BindingError::BindingConflict); }
            tx.commit()?;
            admission.recheck_registered_root()?;
            return Ok(previous);
        }
        let exists: Option<String> = tx.query_row(
            "SELECT workspace_id FROM registered_s3_bindings
             WHERE remote_scope_hash = ?1 OR profile_id = ?2 LIMIT 1",
            params![&scope_hash, profile_id],
            |r| r.get(0),
        ).optional()?;
        if exists.is_some() {
            return Err(S3BindingError::BindingConflict);
        }
        tx.execute(
            "INSERT INTO registered_s3_bindings
             (workspace_id, profile_id, remote_scope_hash, record_json, record_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![proposal.workspace_id.to_string(), profile_id, scope_hash, record_json, digest],
        )?;
        tx.commit()?;
        // A root change after commit creates an ambiguous but retained config,
        // not permission to continue. Never unlink or silently rebind it.
        admission.recheck_registered_root()?;
        Ok(proposal)
    }

    /// On subsequent Core launches use a newly opened registered admission;
    /// the original local file identity and registration must still match.
    pub(crate) fn load_for_registered(
        &self,
        admission: &RegisteredReadAdmission<'_>,
    ) -> Result<Option<RegisteredS3Binding>, S3BindingError> {
        admission.recheck_registered_root()?;
        validate_appdata_location(admission, &self.database_path)?;
        let binding = load(&self.connection, admission.registered_workspace_id())?;
        if let Some(ref binding) = binding {
            let (device, file) = admission.registered_root_identity_for_profile()?;
            if binding.local_root_device != device
                || binding.local_root_file != file
                || binding.local_root_path
                    != admission.registered_root_path().to_str()
                        .ok_or(S3BindingError::InvalidStoreLocation)?
            {
                return Err(S3BindingError::BindingConflict);
            }
        }
        admission.recheck_registered_root()?;
        Ok(binding)
    }
}

fn load(
    connection: &Connection,
    workspace_id: Uuid,
) -> Result<Option<RegisteredS3Binding>, S3BindingError> {
    let row: Option<(String, String, Vec<u8>, String)> = connection.query_row(
        "SELECT profile_id, remote_scope_hash, record_json, record_hash
         FROM registered_s3_bindings WHERE workspace_id = ?1",
        [workspace_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).optional()?;
    let Some((profile_id, scope_hash, json, digest)) = row else { return Ok(None); };
    if blake3::hash(&json).to_hex().as_str() != digest {
        return Err(S3BindingError::CorruptedBinding);
    }
    let binding: RegisteredS3Binding = serde_json::from_slice(&json)
        .map_err(|_| S3BindingError::CorruptedBinding)?;
    if binding.workspace_id != workspace_id
        || binding.profile_id != profile_id
        || binding.scope_hash()? != scope_hash
    {
        return Err(S3BindingError::CorruptedBinding);
    }
    Ok(Some(binding))
}

fn validate_appdata_location(
    admission: &RegisteredReadAdmission<'_>,
    database_path: &Path,
) -> Result<(), S3BindingError> {
    let parent = database_path.parent().ok_or(S3BindingError::InvalidStoreLocation)?;
    let leaf = database_path.file_name().ok_or(S3BindingError::InvalidStoreLocation)?;
    if leaf.is_empty() || leaf == "." || leaf == ".." {
        return Err(S3BindingError::InvalidStoreLocation);
    }
    let canonical_parent = fs::canonicalize(parent)?;
    let registered_root = fs::canonicalize(admission.registered_root_path())?;
    if canonical_parent.starts_with(&registered_root) {
        return Err(S3BindingError::InvalidStoreLocation);
    }
    match fs::symlink_metadata(database_path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.file_type().is_symlink()
                || fs::canonicalize(database_path)?.starts_with(&registered_root)
            {
                return Err(S3BindingError::InvalidStoreLocation);
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.nlink() != 1 {
                    return Err(S3BindingError::InvalidStoreLocation);
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x0000_0400 != 0 {
                    return Err(S3BindingError::InvalidStoreLocation);
                }
                // A hardlink outside the vault can still name the same NTFS
                // file object inside it. Reject multiple links, not just
                // reparse points and pathname aliases.
                let handle = fs::OpenOptions::new().read(true).open(database_path)?;
                let (_, _, links) = super::pinned_windows::file_identity_with_links(&handle)?;
                if links != 1 {
                    return Err(S3BindingError::InvalidStoreLocation);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(S3BindingError::Io(error)),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use crate::sync::{OperationId, S3HeadPointerV1, S3ManifestV1};
    use std::sync::Arc;

    fn core(root: &Path, data: &Path) -> Arc<Core> {
        fs::create_dir_all(root).unwrap();
        fs::create_dir_all(data).unwrap();
        let core = Core::open(data, Arc::new(|_: CoreEvent| {}));
        core.ui_state.resolve_workspace(root.to_str().unwrap(), 1).unwrap();
        core
    }

    fn s3() -> (S3ConnectionProfile, S3Keyspace, S3VerifiedHead) {
        let namespace = S3Keyspace::new("akula/shared", VaultId::new()).unwrap();
        let profile = S3ConnectionProfile {
            bucket: "akula-binding-test".into(),
            region: "us-east-1".into(),
            endpoint: Some("http://127.0.0.1:9000".into()),
            force_path_style: true,
        };
        let manifest = S3ManifestV1::new(
            &namespace, OperationId::new(), 1, None, vec![],
        ).unwrap();
        let pointer = S3HeadPointerV1::new(
            &namespace, 1, manifest.manifest_id, &manifest.digest(&namespace).unwrap(), None,
        ).unwrap();
        (profile, namespace, S3VerifiedHead {
            pointer,
            etag: "\"simulated-provider-etag\"".into(),
            manifest,
        })
    }

    #[test]
    fn two_independent_local_registrations_bind_same_remote_vault_without_id_alias() {
        let temp = tempfile::tempdir().unwrap();
        let root_a = temp.path().join("one");
        let root_b = temp.path().join("two");
        let data_a = temp.path().join("app-a");
        let data_b = temp.path().join("app-b");
        let a = core(&root_a, &data_a);
        let b = core(&root_b, &data_b);
        let id_a = a.ui_state.list_workspaces().unwrap()[0].id;
        let id_b = b.ui_state.list_workspaces().unwrap()[0].id;
        assert_ne!(id_a, id_b);
        let (profile, namespace, head) = s3();

        {
            let admitted_a = RegisteredReadAdmission::open(&a, &root_a).unwrap();
            let admitted_b = RegisteredReadAdmission::open(&b, &root_b).unwrap();
            let mut store_a = SqliteRegisteredS3Bindings::open(
                &admitted_a, &data_a.join("s3-profiles.sqlite")).unwrap();
            let mut store_b = SqliteRegisteredS3Bindings::open(
                &admitted_b, &data_b.join("s3-profiles.sqlite")).unwrap();
            let joined_a = store_a.bind_existing_verified_s3(
                &admitted_a, "s3-vault", &profile, &namespace, &head,
            ).unwrap();
            let joined_b = store_b.bind_existing_verified_s3(
                &admitted_b, "s3-vault", &profile, &namespace, &head,
            ).unwrap();
            assert_ne!(joined_a.workspace_id, joined_b.workspace_id);
            assert_eq!(joined_a.vault_id, joined_b.vault_id);
            assert_ne!(joined_a.workspace_id, joined_a.vault_id.as_uuid());
            assert_ne!(joined_b.workspace_id, joined_b.vault_id.as_uuid());
            assert!(joined_a.matches_remote(&profile, &namespace));
            assert_eq!(store_a.bind_existing_verified_s3(
                &admitted_a, "s3-vault", &profile, &namespace, &head,
            ).unwrap(), joined_a);
        }
        // Simulate real Core process restart; the app-data record is stable,
        // and the root's local OS identity must remain unchanged.
        drop(a);
        drop(b);
        let a = Core::open(&data_a, Arc::new(|_: CoreEvent| {}));
        let b = Core::open(&data_b, Arc::new(|_: CoreEvent| {}));
        for (core, root, data, expected_id) in [
            (&a, &root_a, &data_a, id_a),
            (&b, &root_b, &data_b, id_b),
        ] {
            let admitted = RegisteredReadAdmission::open(core, root).unwrap();
            let store = SqliteRegisteredS3Bindings::open(
                &admitted, &data.join("s3-profiles.sqlite")).unwrap();
            let binding = store.load_for_registered(&admitted).unwrap().unwrap();
            assert_eq!(binding.workspace_id, expected_id);
            assert_eq!(binding.vault_id, namespace.vault_id());
        }
    }

    #[test]
    fn rebinding_or_shared_scope_reuse_in_one_appdata_is_denied() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("shared-app-data");
        let a = core(&temp.path().join("first"), &data);
        let b = core(&temp.path().join("second"), &data);
        let (profile, namespace, head) = s3();
        let first_root = temp.path().join("first");
        let second_root = temp.path().join("second");
        let guard_a = RegisteredReadAdmission::open(&a, &first_root).unwrap();
        let guard_b = RegisteredReadAdmission::open(&b, &second_root).unwrap();
        let mut store = SqliteRegisteredS3Bindings::open(
            &guard_a, &data.join("bindings.sqlite")).unwrap();
        store.bind_existing_verified_s3(
            &guard_a, "first-profile", &profile, &namespace, &head,
        ).unwrap();
        assert!(matches!(store.bind_existing_verified_s3(
            &guard_b, "second-profile", &profile, &namespace, &head,
        ), Err(S3BindingError::BindingConflict)));
        let mut switched = profile.clone();
        switched.bucket = "different-bucket".into();
        assert!(matches!(store.bind_existing_verified_s3(
            &guard_a, "first-profile", &switched, &namespace, &head,
        ), Err(S3BindingError::BindingConflict)));
        assert_eq!(store.load_for_registered(&guard_a).unwrap().unwrap().bucket,
            profile.bucket);
    }

    #[test]
    fn wrong_remote_root_and_database_inside_vault_fail_without_binding() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let data = temp.path().join("appdata");
        let core = core(&root, &data);
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        assert!(matches!(SqliteRegisteredS3Bindings::open(
            &guard, &root.join("unsafe.sqlite"),
        ), Err(S3BindingError::InvalidStoreLocation)));
        assert!(!root.join("unsafe.sqlite").exists());

        let (profile, ns, mut head) = s3();
        let mut store = SqliteRegisteredS3Bindings::open(
            &guard, &data.join("bindings.sqlite")).unwrap();
        head.pointer = S3HeadPointerV1::new(
            &ns, 1, OperationId::new(), &head.manifest.digest(&ns).unwrap(), None,
        ).unwrap();
        assert!(matches!(store.bind_existing_verified_s3(
            &guard, "remote", &profile, &ns, &head,
        ), Err(S3BindingError::UnverifiedHead)));
        assert!(store.load_for_registered(&guard).unwrap().is_none());
    }

    #[test]
    fn corrupt_binding_json_fails_closed_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let data = temp.path().join("appdata");
        let core = core(&root, &data);
        let guard = RegisteredReadAdmission::open(&core, &root).unwrap();
        let (profile, ns, head) = s3();
        let db = data.join("bindings.sqlite");
        {
            let mut store = SqliteRegisteredS3Bindings::open(&guard, &db).unwrap();
            store.bind_existing_verified_s3(
                &guard, "remote", &profile, &ns, &head,
            ).unwrap();
        }
        let conn = Connection::open(&db).unwrap();
        conn.execute("UPDATE registered_s3_bindings SET record_json = ?1",
            [b"tampered".as_slice()]).unwrap();
        drop(conn);
        let store = SqliteRegisteredS3Bindings::open(&guard, &db).unwrap();
        assert!(matches!(store.load_for_registered(&guard),
            Err(S3BindingError::CorruptedBinding)));
    }
}
