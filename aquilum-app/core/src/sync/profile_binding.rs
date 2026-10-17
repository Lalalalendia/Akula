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
    fn verified_s3_vault_id_is_persistently_linked_to_existing_external_lock() {
        use super::super::filegate_custody_identity::SqliteCustodyIdentityStore;
        use super::super::filegate_lock_custody::scoped_leaf;
        use super::super::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("local-vault");
        let appdata = temp.path().join("host-data");
        let core = core(&root, &appdata);
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let anchor = temp.path().join("external-locks")
            .join(scoped_leaf(PinnedProbeRoot::open(&root).unwrap().identity()));
        fs::create_dir_all(&anchor).unwrap();
        let lock = anchor.join(LOCK_NAME);
        fs::write(&lock, b"pre-existing-lock-bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&anchor, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock, fs::Permissions::from_mode(0o600)).unwrap();
        }
        #[cfg(unix)]
        let anchor = fs::canonicalize(&anchor).unwrap();

        let (profile, keyspace, head) = s3();
        let profiles_db = appdata.join("registered-s3.sqlite");
        let custody_db = appdata.join("custody-binding.sqlite");
        {
            let mut profiles = SqliteRegisteredS3Bindings::open(&admission, &profiles_db).unwrap();
            let joined = profiles.bind_existing_verified_s3(
                &admission, "my-s3-profile", &profile, &keyspace, &head,
            ).unwrap();
            let mut custody = SqliteCustodyIdentityStore::open(&admission, &custody_db).unwrap();
            let recorded = custody.bind_registered_s3(&admission, &profiles, &anchor).unwrap();
            assert_eq!(recorded.vault_id, joined.vault_id);
            assert_eq!(recorded.vault_id, keyspace.vault_id());
            assert_ne!(recorded.workspace_id, recorded.vault_id.as_uuid());
        }

        drop(admission);
        drop(core);
        let restarted = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        let admission = RegisteredReadAdmission::open(&restarted, &root).unwrap();
        let profiles = SqliteRegisteredS3Bindings::open(&admission, &profiles_db).unwrap();
        let custody = SqliteCustodyIdentityStore::open(&admission, &custody_db).unwrap();
        let verified = custody.verify_registered_s3(&admission, &profiles, &anchor).unwrap();
        assert_eq!(verified.vault_id, keyspace.vault_id());
        assert_eq!(fs::read(lock).unwrap(), b"pre-existing-lock-bytes");
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

    /// Runs in a REAL second OS process when explicitly targeted by the
    /// parent fixture. Normal unit-test runs do not take any lock.
    #[test]
    fn s3_interprocess_child_holds_external_custody_lock() {
        let Some(scope) = std::env::var_os("AKULA_SYNC_S3_CUSTODY_CHILD_SCOPE") else {
            return;
        };
        let signal = std::path::PathBuf::from(
            std::env::var_os("AKULA_SYNC_S3_CUSTODY_CHILD_SIGNAL").unwrap(),
        );
        let root = super::super::filegate_lock_probe::PinnedProbeRoot::open(
            std::path::Path::new(&scope),
        ).unwrap();
        let _held = root.try_lock().unwrap().expect("independent child acquires lock");
        fs::write(signal, b"held").unwrap();
        loop {
            std::thread::park_timeout(std::time::Duration::from_secs(60));
        }
    }

    #[test]
    fn bound_remote_vault_can_add_file_without_aliasing_local_registration() {
        use crate::sync::{
            DocumentId, JournalAction, JournalPhase, OperationIntent, RevisionId,
            SqliteDagStore, SqliteSyncJournal, SyncRelativePath,
        };
        use super::super::filegate_custody_identity::SqliteCustodyIdentityStore;
        use super::super::filegate_lock_custody::scoped_leaf;
        use super::super::filegate_lock_probe::{PinnedProbeRoot, LOCK_NAME};

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        let appdata = temp.path().join("appdata");
        let core = core(&root, &appdata);
        fs::create_dir_all(root.join("notes")).unwrap();
        let admission = RegisteredReadAdmission::open(&core, &root).unwrap();
        let (profile, namespace, head) = s3();
        assert_ne!(admission.registered_workspace_id(), namespace.vault_id().as_uuid());

        let mut bindings = SqliteRegisteredS3Bindings::open(
            &admission, &appdata.join("profiles.sqlite"),
        ).unwrap();
        bindings.bind_existing_verified_s3(
            &admission, "joined-cloud", &profile, &namespace, &head,
        ).unwrap();

        // A verified cloud profile alone MUST NOT unlock local writes. An
        // independent, pre-existing outside-vault OS lock has its own durable
        // binding to this exact physical root and shared cloud VaultId.
        let anchor = temp.path().join("os-custody")
            .join(scoped_leaf(PinnedProbeRoot::open(&root).unwrap().identity()));
        fs::create_dir_all(&anchor).unwrap();
        let lock_file = anchor.join(LOCK_NAME);
        fs::write(&lock_file, b"pre-existing, never replaced by Akula").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&anchor, fs::Permissions::from_mode(0o700)).unwrap();
            fs::set_permissions(&lock_file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        #[cfg(unix)]
        let anchor = fs::canonicalize(anchor).unwrap();
        let mut custody = SqliteCustodyIdentityStore::open(
            &admission, &appdata.join("custody.sqlite"),
        ).unwrap();
        custody.bind_registered_s3(&admission, &bindings, &anchor).unwrap();

        let bytes = "Новая глава из S3\n".as_bytes();
        let hash = blake3::hash(bytes).to_hex().to_string();
        let path = SyncRelativePath::parse("notes/new.md").unwrap();
        let document = DocumentId::new();
        let operation_id = OperationId::new();
        let intent = OperationIntent {
            operation_id,
            vault_id: namespace.vault_id(),
            document_id: document,
            profile_id: "joined-cloud".into(),
            revision_id: RevisionId::new(),
            action: JournalAction::StageDownload,
            path: path.clone(),
            remote_object_id: namespace.immutable_blob_key(&hash).unwrap(),
            remote_guard: None,
            expected_content_hash: hash.clone(),
            expected_baseline: None,
        };
        let mut journal = SqliteSyncJournal::open(&appdata.join("ops.sqlite")).unwrap();
        let mut cache = SqliteDagStore::open(&appdata.join("blobs.sqlite")).unwrap();
        assert_eq!(cache.store_blob(bytes).unwrap(), hash);
        journal.begin(intent).unwrap();
        journal.mark_staged(operation_id, bytes).unwrap();

        // The legacy local-only path MUST still reject a remote vault UUID.
        assert!(matches!(
            admission.create_staged_if_absent(operation_id, &journal, &cache),
            Err(super::super::host_admission::LocalCreateError::InvalidStagedIntent)
        ));
        assert!(!root.join(path.as_str()).exists());

        // Wrong remote configuration is never a substitute for the binding.
        let mut wrong_profile = profile.clone();
        wrong_profile.force_path_style = false;
        assert!(admission.create_staged_if_absent_for_registered_s3(
            operation_id, &journal, &cache, &bindings, &wrong_profile, &namespace,
            &custody, &anchor,
        ).is_err());
        assert!(!root.join(path.as_str()).exists());

        // A REAL independent process holds the SAME physical external lock.
        // The candidate Akula must decline BEFORE recording any one-shot
        // native attempt and without creating either the stage or final name.
        struct StopChild(std::process::Child);
        impl Drop for StopChild {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let report = temp.path().join("independent-lock-held");
        let mut child = StopChild(
            std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("sync::profile_binding::tests::s3_interprocess_child_holds_external_custody_lock")
                .arg("--nocapture")
                .env("AKULA_SYNC_S3_CUSTODY_CHILD_SCOPE", &anchor)
                .env("AKULA_SYNC_S3_CUSTODY_CHILD_SIGNAL", &report)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn().unwrap()
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !report.exists() {
            assert!(std::time::Instant::now() < deadline, "child lock barrier timed out");
            assert!(child.0.try_wait().unwrap().is_none(), "child died before OS-lock acquisition");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(fs::read(&report).unwrap(), b"held");
        assert!(matches!(
            admission.create_staged_if_absent_for_registered_s3(
                operation_id, &journal, &cache, &bindings, &profile, &namespace,
                &custody, &anchor,
            ),
            Err(super::super::host_admission::LocalCreateError::Admission(
                VaultPortError::PermissionDenied
            ))
        ));
        assert!(journal.local_publication_review_queue().unwrap().is_empty());
        assert!(!root.join(path.as_str()).exists());
        drop(child); // kernel drops the child's held OS-lock handles.

        // An existing SQLite custody checksum cannot authorize a NEW file
        // object that replaced the trusted OS lock pathname. Never rebind it
        // implicitly and never consume the one-shot local publication ticket.
        let preserved_lock = anchor.join("original-lock-retained");
        fs::rename(&lock_file, &preserved_lock).unwrap();
        fs::write(&lock_file, b"new physical inode, same familiar name").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&lock_file, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(admission.create_staged_if_absent_for_registered_s3(
            operation_id, &journal, &cache, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).is_err());
        assert!(journal.local_publication_review_queue().unwrap().is_empty());
        assert!(!root.join(path.as_str()).exists());
        fs::remove_file(&lock_file).unwrap();
        fs::rename(&preserved_lock, &lock_file).unwrap();

        // Only the exact registered profile + immutable blob key and the
        // pinned, held external lock can enter the native no-replace path.
        let published = admission.create_staged_if_absent_for_registered_s3(
            operation_id, &journal, &cache, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(published.byte_length, bytes.len() as u64);
        assert_eq!(fs::read(root.join(path.as_str())).unwrap(), bytes);
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.baseline(namespace.vault_id(), document).unwrap().is_none());
        assert!(admission.create_staged_if_absent_for_registered_s3(
            operation_id, &journal, &cache, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).is_err());
        assert_eq!(fs::read(root.join(path.as_str())).unwrap(), bytes);

        // The old local-UUID recovery API must still refuse a remote-vault
        // publication. Matching bytes alone cannot bypass the scope binding.
        assert!(admission.inspect_local_publication(operation_id, &journal).is_err());
        assert!(admission.record_local_recovery_evidence(operation_id, &journal).is_err());
        assert!(journal.read_local_recovery_case(operation_id).unwrap().is_none());
        assert!(admission.inspect_local_publication_for_registered_s3(
            operation_id, &journal, &bindings, &wrong_profile, &namespace,
        ).is_err());

        let observed = admission.inspect_local_publication_for_registered_s3(
            operation_id, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(observed.assessment,
            crate::sync::local_recovery::LocalRecoveryAssessment::FinalNameMatchesExpected);
        let case = admission.record_local_recovery_evidence_for_registered_s3(
            operation_id, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(case.vault_id, namespace.vault_id());
        assert_eq!(case.evidence, observed);
        assert!(admission.record_local_recovery_evidence_for_registered_s3(
            operation_id, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).is_err(), "forensic evidence is write-once");

        // Archive a COPY of the final bytes, not the vault file. It remains
        // app-private and does not release the original one-shot claim.
        let archived = admission.archive_local_recovery_copy_for_registered_s3(
            operation_id, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(archived.stage_bytes, None);
        assert_eq!(archived.final_bytes.as_deref(), Some(bytes));
        let preview = admission.recovery_operator_preview_for_registered_s3(
            operation_id, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(preview.vault_id, namespace.vault_id());
        assert_eq!(preview.review_count, 0);
        assert!(admission.recovery_operator_preview(operation_id, &journal).is_err());
        let reviewed = admission.record_recovery_operator_review_for_registered_s3(
            operation_id, &preview.path, &preview.archive_fingerprint,
            preview.review_count, &preview.latest_event_digest,
            crate::sync::recovery_review::RecoveryReviewDisposition::KeepBlocked,
            "Keep blocked until native receipt is independently verified",
            &mut journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(reviewed.sequence, 1);
        assert!(admission.record_recovery_operator_review_for_registered_s3(
            operation_id, &preview.path, &preview.archive_fingerprint,
            preview.review_count, &preview.latest_event_digest,
            crate::sync::recovery_review::RecoveryReviewDisposition::KeepBlocked,
            "Stale confirmation must not replace the review",
            &mut journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).is_err());

        // A process may also die BEFORE the no-replace rename. Simulate a
        // verified, preclaimed stage with no final name. It is an incident,
        // not permission to replay or silently complete the publication.
        let unfinished_document = DocumentId::new();
        let unfinished_op = OperationId::new();
        let unfinished_path = SyncRelativePath::parse("notes/unfinished.md").unwrap();
        let unfinished_intent = OperationIntent {
            operation_id: unfinished_op,
            vault_id: namespace.vault_id(),
            document_id: unfinished_document,
            profile_id: "joined-cloud".into(),
            revision_id: RevisionId::new(),
            action: JournalAction::StageDownload,
            path: unfinished_path.clone(),
            remote_object_id: namespace.immutable_blob_key(&hash).unwrap(),
            remote_guard: None,
            expected_content_hash: hash.clone(),
            expected_baseline: None,
        };
        journal.begin(unfinished_intent.clone()).unwrap();
        journal.mark_staged(unfinished_op, bytes).unwrap();
        journal.record_local_publication_attempt_once(&unfinished_intent).unwrap();
        let stage_path = root.join("notes")
            .join(format!(".akula-sync-stage-{unfinished_op}"));
        fs::write(&stage_path, bytes).unwrap();
        let staged_observation = admission.inspect_local_publication_for_registered_s3(
            unfinished_op, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(staged_observation.assessment,
            crate::sync::local_recovery::LocalRecoveryAssessment::StagedOnly);
        let stage_case = admission.record_local_recovery_evidence_for_registered_s3(
            unfinished_op, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(stage_case.evidence, staged_observation);
        let stage_archive = admission.archive_local_recovery_copy_for_registered_s3(
            unfinished_op, &journal, &bindings, &profile, &namespace,
            &custody, &anchor,
        ).unwrap();
        assert_eq!(stage_archive.stage_bytes.as_deref(), Some(bytes));
        assert_eq!(stage_archive.final_bytes, None);
        assert!(!root.join(unfinished_path.as_str()).exists());
        assert_eq!(fs::read(&stage_path).unwrap(), bytes);
        assert!(journal.baseline(namespace.vault_id(), unfinished_document).unwrap().is_none());

        // Neither repeated reads nor a recorded operator review are a
        // File Gate authority, a confirmed baseline or a delete capability.
        assert_eq!(fs::read(root.join(path.as_str())).unwrap(), bytes);
        assert_eq!(journal.get(operation_id).unwrap().unwrap().phase, JournalPhase::Staged);
        assert!(journal.baseline(namespace.vault_id(), document).unwrap().is_none());

        drop(admission);
        drop(bindings);
        drop(cache);
        drop(journal);
        drop(core);
        let reopened_core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        let guard = RegisteredReadAdmission::open(&reopened_core, &root).unwrap();
        let recovered_bindings = SqliteRegisteredS3Bindings::open(
            &guard, &appdata.join("profiles.sqlite"),
        ).unwrap();
        let recovered_journal = SqliteSyncJournal::open(&appdata.join("ops.sqlite")).unwrap();
        let reopened = guard.recovery_operator_preview_for_registered_s3(
            operation_id, &recovered_journal, &recovered_bindings, &profile, &namespace,
        ).unwrap();
        assert_eq!(reopened.review_count, 1);
        assert_eq!(reopened.vault_id, namespace.vault_id());
        assert!(recovered_journal.baseline(namespace.vault_id(), document).unwrap().is_none());
        assert_eq!(fs::read(root.join(path.as_str())).unwrap(), bytes);
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
