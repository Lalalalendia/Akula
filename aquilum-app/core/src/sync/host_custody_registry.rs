//! SYNC-P0-A: installer-owned shared custody registry, independent of appdata.
//!
//! The normal Core File Gate and HistoryService do NOT consult this yet. This
//! module deliberately provides only fail-closed durable *registration
//! evidence*. It cannot authorize a write, clear HOLD, grant S3 CAS, or make
//! same-user hostile processes trustworthy. The next P0-B must carry one
//! verified host source into EVERY cooperative Core/HistoryService admission
//! and hold an external OS epoch through primary bytes plus history.
//!
//! A Core must never create this database on a missing-file read. The host
//! installer explicitly provisions it once at a stable, trusted path outside
//! all vaults and every private appdata. A missing, damaged or unexpected
//! schema is an error, not an empty enrollment roster.

use super::VaultId;
use rusqlite::{params, Connection, OpenFlags, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::Path;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const SCHEMA_VERSION: i64 = 1;
const MAX_RECORD_BYTES: usize = 4096;
const MAX_HOST_RECORDS: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhysicalObjectId {
    pub device: u64,
    pub file: u64,
}

impl PhysicalObjectId {
    fn valid(self) -> bool { self.file != 0 }
    fn key(self) -> String { format!("{:016x}:{:016x}", self.device, self.file) }
}

/// All identifiers must come from independently pinned host read/OS-lock
/// admissions, never from arbitrary JSON supplied by an S3 server.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HostCustodyRecord {
    pub root_path: String,
    pub root_identity: PhysicalObjectId,
    pub vault_id: VaultId,
    pub profile_id: String,
    pub custody_scope_path: String,
    pub lock_identity: PhysicalObjectId,
}

impl HostCustodyRecord {
    fn strict_validate(&self) -> bool {
        !self.root_path.is_empty()
            && self.root_path.len() <= 2048
            && Path::new(&self.root_path).is_absolute()
            && self.root_identity.valid()
            && self.lock_identity.valid()
            && !self.custody_scope_path.is_empty()
            && self.custody_scope_path.len() <= 2048
            && Path::new(&self.custody_scope_path).is_absolute()
            && !Path::new(&self.custody_scope_path).starts_with(&self.root_path)
            && self.profile_id.len() <= 96
            && !self.profile_id.is_empty()
            && self.profile_id.bytes().all(|c| c.is_ascii_lowercase()
                || c.is_ascii_digit() || c == b'-' || c == b'_')
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum HostCustodyError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    InvalidHostLocation,
    UnsupportedSchema,
    DamagedRegistry,
    InvalidRecord,
    ConflictingEnrollment,
}

impl From<std::io::Error> for HostCustodyError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}
impl From<rusqlite::Error> for HostCustodyError {
    fn from(e: rusqlite::Error) -> Self { Self::Sqlite(e) }
}

#[allow(dead_code)]
pub(crate) struct SharedHostCustodyRegistry {
    conn: Connection,
}

#[allow(dead_code)]
// Preliminary namespace rejection only. This guards the *immediate*
// host-ledger parent against a symlink/junction substitution, but does NOT
// pin ancestor directories, inode identity, owner/ACL or SQLite WAL/SHM.
fn checked_host_parent(path: &Path) -> Result<&Path, HostCustodyError> {
    let parent = path.parent().ok_or(HostCustodyError::InvalidHostLocation)?;
    let entry = fs::symlink_metadata(parent)?;
    if !entry.is_dir() || entry.file_type().is_symlink() {
        return Err(HostCustodyError::InvalidHostLocation);
    }
    #[cfg(windows)]
    if {
        use std::os::windows::fs::MetadataExt;
        entry.file_attributes() & 0x0000_0400 != 0
    } {
        // Windows directory reparse points (including junctions) cannot
        // redirect a compiler-pinned host ledger to a private empty DB.
        return Err(HostCustodyError::InvalidHostLocation);
    }
    Ok(parent)
}

impl SharedHostCustodyRegistry {
    /// INSTALLER ONLY. Provision in a *pre-existing*, non-symlink host
    /// directory, and explicitly prohibit creating it in any local workspace
    /// or appdata. Never replace an existing file, including a torn database.
    /// The installer must additionally enforce its platform ACL/owner policy;
    /// this API alone is not a privileged trust root.
    pub(crate) fn provision_new(
        path: &Path, forbidden_roots: &[&Path],
    ) -> Result<Self, HostCustodyError> {
        let parent = fs::canonicalize(checked_host_parent(path)?)?;
        for forbidden in forbidden_roots {
            let other = fs::canonicalize(forbidden)?;
            if parent.starts_with(other) {
                return Err(HostCustodyError::InvalidHostLocation);
            }
        }
        // create_new means a failed/interrupted install cannot erase a
        // previous, potentially valid registration or silently start anew.
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        drop(file);
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        configure(&conn)?;
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE shared_custody (
                 root_key TEXT PRIMARY KEY NOT NULL,
                 root_path TEXT NOT NULL UNIQUE,
                 record_json BLOB NOT NULL,
                 record_blake3 TEXT NOT NULL
             );
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
        Ok(Self { conn })
    }

    /// Ordinary Core must use this method: NEVER open with CREATE, auto
    /// migrate, automatically clear corruption, or infer an empty registry
    /// from a missing shared host database.
    pub(crate) fn open_existing(path: &Path) -> Result<Self, HostCustodyError> {
        // Never follow a substituted parent alias into another, freshly
        // provisioned but empty registry. Missing/untrusted path stays HOLD.
        checked_host_parent(path)?;
        let entry = fs::symlink_metadata(path)?;
        if !entry.is_file() || entry.file_type().is_symlink() {
            return Err(HostCustodyError::InvalidHostLocation);
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        configure(&conn)?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(HostCustodyError::UnsupportedSchema);
        }
        let check: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(HostCustodyError::DamagedRegistry);
        }
        // Validate ALL durable rows, not just those matching a later
        // requested workspace. A corrupted root_key/path could otherwise
        // disappear from a lookup and be mistaken for "not enrolled".
        let mut count = 0usize;
        {
            let mut statement = conn.prepare(
                "SELECT root_key, root_path, record_json, record_blake3 FROM shared_custody"
            )?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                count += 1;
                if count > MAX_HOST_RECORDS {
                    return Err(HostCustodyError::DamagedRegistry);
                }
                decode_row(row)?;
            }
        }
        Ok(Self { conn })
    }

    /// Insert-only registration by physically identified root, with
    /// immutable VaultId/profile and exact OS lock object. Reopening or
    /// retrying the exact same record is idempotent. The SQLite transaction
    /// is WAL/FULL; missing ACK must be followed by readback, never replace.
    pub(crate) fn enroll(
        &mut self, record: &HostCustodyRecord,
    ) -> Result<(), HostCustodyError> {
        if !record.strict_validate() {
            return Err(HostCustodyError::InvalidRecord);
        }
        let bytes = serde_json::to_vec(record)
            .map_err(|_| HostCustodyError::InvalidRecord)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(HostCustodyError::InvalidRecord);
        }
        let key = record.root_identity.key();
        let digest = blake3::hash(&bytes).to_hex().to_string();
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = lookup(&tx, &key, &record.root_path)?;
        if let Some(previous) = existing {
            if previous != *record {
                return Err(HostCustodyError::ConflictingEnrollment);
            }
            tx.commit()?;
            return Ok(());
        }
        // The same physical host registry must not register a parent and a
        // nested workspace as independently authorized S3 participants.
        // Check INSIDE the same BEGIN IMMEDIATE used by ordinary writers:
        // alternate appdata and different inode identities cannot race in
        // a conflicting hierarchy after both received positive enrollment.
        if has_registered_path_overlap(&tx, Path::new(&record.root_path))? {
            return Err(HostCustodyError::ConflictingEnrollment);
        }
        tx.execute(
            "INSERT INTO shared_custody(root_key, root_path, record_json, record_blake3)
             VALUES (?1, ?2, ?3, ?4)",
            params![key, record.root_path, bytes, digest],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// A cooperating host-configured admission epoch, NOT write permission.
    /// BEGIN IMMEDIATE takes the SAME host-wide SQLite writer lock as enroll.
    /// The caller must retain this lease over primary bytes AND HistoryService
    /// and separately enforce physical root pinning and all File Gate policy.
    /// Normal Core does not use this yet; PR #251 remains release-blocking.
    pub(crate) fn with_unenrolled_epoch<T>(
        &mut self,
        root_path: &Path,
        root_identity: PhysicalObjectId,
        operation: impl FnOnce() -> T,
    ) -> Result<T, HostCustodyError> {
        if !root_identity.valid() {
            return Err(HostCustodyError::InvalidRecord);
        }
        let root = normalized_root_for_lookup(root_path)?;
        let transaction =
            self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // Enrollment cannot commit between lookup and the full publication.
        if lookup(&transaction, &root_identity.key(), &root)?.is_some() {
            return Err(HostCustodyError::ConflictingEnrollment);
        }
        // The exact same, component-boundary overlap invariant must be used
        // by BOTH enrollment and ordinary File Gate publication.
        if has_registered_path_overlap(&transaction, Path::new(&root))? {
            return Err(HostCustodyError::ConflictingEnrollment);
        }
        let outcome = operation();
        transaction.commit()?;
        Ok(outcome)
    }

    /// A checked read only. A missing record is never write authorization.
    pub(crate) fn inspect(
        &self, root_path: &Path, root_identity: PhysicalObjectId,
    ) -> Result<Option<HostCustodyRecord>, HostCustodyError> {
        if !root_identity.valid() {
            return Err(HostCustodyError::InvalidRecord);
        }
        let root = normalized_root_for_lookup(root_path)?;
        let found = lookup(&self.conn, &root_identity.key(), &root)?;
        if let Some(ref record) = found {
            if record.root_path != root || record.root_identity != root_identity {
                return Err(HostCustodyError::ConflictingEnrollment);
            }
        }
        Ok(found)
    }
}


fn normalized_root_for_lookup(root_path: &Path) -> Result<String, HostCustodyError> {
    // Refuse symlink/junction leaves; a full production admission must also
    // pin trusted host directory and ancestry handles.
    let selected = fs::symlink_metadata(root_path)?;
    if !selected.is_dir() || selected.file_type().is_symlink() {
        return Err(HostCustodyError::InvalidRecord);
    }
    #[cfg(windows)]
    if {
        use std::os::windows::fs::MetadataExt;
        selected.file_attributes() & 0x0000_0400 != 0
    } {
        return Err(HostCustodyError::InvalidRecord);
    }
    // macOS /var and /private/var, and Windows extended path spellings,
    // must never become alternate, falsely-unregistered lookup keys.
    let physical_path = fs::canonicalize(root_path)?;
    Ok(physical_path.to_str()
        .ok_or(HostCustodyError::InvalidRecord)?.to_owned())
}

fn configure(conn: &Connection) -> Result<(), HostCustodyError> {
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = FULL;
         PRAGMA busy_timeout = 5000;
         PRAGMA foreign_keys = ON;",
    )?;
    Ok(())
}

/// Canonical host roots may have distinct device:file identities while
/// overlapping as parent/child paths. Consult every registered root inside
/// the caller's BEGIN IMMEDIATE transaction; never infer authority from an
/// exact-only UUID or inode lookup. strip_root compares whole path components
/// and normalizes Windows case/extended prefixes and Unicode spellings.
fn has_registered_path_overlap(
    conn: &Connection,
    candidate: &Path,
) -> Result<bool, HostCustodyError> {
    let mut statement = conn.prepare("SELECT root_path FROM shared_custody")?;
    let entries = statement.query_map([], |row| row.get::<_, String>(0))?;
    for entry in entries {
        let existing = entry?;
        let existing = Path::new(&existing);
        if crate::search::paths::strip_root(candidate, existing).is_some()
            || crate::search::paths::strip_root(existing, candidate).is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn lookup(
    conn: &Connection, root_key: &str, root_path: &str,
) -> Result<Option<HostCustodyRecord>, HostCustodyError> {
    let mut statement = conn.prepare(
        "SELECT root_key, root_path, record_json, record_blake3
         FROM shared_custody WHERE root_key = ?1 OR root_path = ?2 LIMIT 2"
    )?;
    let mut rows = statement.query(params![root_key, root_path])?;
    let Some(row) = rows.next()? else { return Ok(None) };
    let record = decode_row(row)?;
    if rows.next()?.is_some() {
        return Err(HostCustodyError::DamagedRegistry);
    }
    Ok(Some(record))
}

/// Check both indexed columns against the complete canonical durable record.
/// BLAKE3 is accidental-corruption detection, not hostile-user authentication.
fn decode_row(row: &rusqlite::Row<'_>) -> Result<HostCustodyRecord, HostCustodyError> {
    let stored_key: String = row.get(0)?;
    let stored_path: String = row.get(1)?;
    let bytes: Vec<u8> = row.get(2)?;
    let digest: String = row.get(3)?;
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES
        || blake3::hash(&bytes).to_hex().as_str() != digest
    {
        return Err(HostCustodyError::DamagedRegistry);
    }
    let record: HostCustodyRecord = serde_json::from_slice(&bytes)
        .map_err(|_| HostCustodyError::DamagedRegistry)?;
    if !record.strict_validate()
        || record.root_identity.key() != stored_key
        || record.root_path != stored_path
        || serde_json::to_vec(&record).ok().as_deref() != Some(bytes.as_slice())
    {
        return Err(HostCustodyError::DamagedRegistry);
    }
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn specimen() -> (TempDir, std::path::PathBuf, std::path::PathBuf,
                       std::path::PathBuf, HostCustodyRecord) {
        let temp = tempfile::tempdir().unwrap();
        let host = temp.path().join("shared-host");
        let vault = temp.path().join("vault");
        let appdata = temp.path().join("appdata-a");
        let scope = temp.path().join("external-OS-lock-scope");
        for dir in [&host, &vault, &appdata, &scope] { fs::create_dir(dir).unwrap(); }
        let database = host.join("approved.sqlite");
        let record = HostCustodyRecord {
            root_path: fs::canonicalize(&vault).unwrap().to_string_lossy().into_owned(),
            root_identity: PhysicalObjectId { device: 1001, file: 9001 },
            vault_id: VaultId::new(),
            profile_id: "approved-s3".into(),
            custody_scope_path: fs::canonicalize(scope).unwrap().to_string_lossy().into_owned(),
            lock_identity: PhysicalObjectId { device: 1001, file: 2209 },
        };
        (temp, database, vault, appdata, record)
    }

    #[test]
    fn missing_install_refuses_and_does_not_create_database() {
        let (_temp, db, _vault, _app, _) = specimen();
        assert!(SharedHostCustodyRegistry::open_existing(&db).is_err());
        assert!(!db.exists());
    }

    #[test]
    fn host_install_is_outside_private_appdata_and_never_reinitializes() {
        let (_temp, db, vault, appdata, _) = specimen();
        assert!(SharedHostCustodyRegistry::provision_new(
            &appdata.join("wrong.sqlite"), &[vault.as_path(), appdata.as_path()],
        ).is_err());
        assert!(!appdata.join("wrong.sqlite").exists());
        let _owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        assert!(SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).is_err());
    }

    #[test]
    fn separate_appdata_reader_observes_enrollment_after_cold_reopen() {
        let (_temp, db, vault, appdata, record) = specimen();
        let mut owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        owner.enroll(&record).unwrap();
        owner.enroll(&record).unwrap();
        drop(owner);
        // Represents another independently opened Core/appdata: the key is
        // the physical root+shared host DB, not A's local workspace UUID.
        let reader = SharedHostCustodyRegistry::open_existing(&db).unwrap();
        assert_eq!(reader.inspect(Path::new(&record.root_path), record.root_identity)
            .unwrap(), Some(record));
    }

    /// Real separate OS process: no private appdata is consulted when
    /// reopening the installer-owned shared ledger. This only proves shared
    /// registry reachability, not an enrolled File Gate writer refusal.
    #[test]
    fn host_custody_process_child_entrypoint() {
        let Some(db) = std::env::var_os("AKULA_HOST_CUSTODY_CHILD_DB") else {
            return;
        };
        let root = std::env::var_os("AKULA_HOST_CUSTODY_CHILD_ROOT").unwrap();
        if let Some(json) = std::env::var_os("AKULA_HOST_CUSTODY_CHILD_ENROLL_JSON") {
            let record: HostCustodyRecord =
                serde_json::from_str(&json.to_string_lossy()).unwrap();
            let report = std::path::PathBuf::from(
                std::env::var_os("AKULA_HOST_CUSTODY_CHILD_ENROLL_REPORT").unwrap()
            );
            let mut registry = SharedHostCustodyRegistry::open_existing(Path::new(&db)).unwrap();
            assert_eq!(fs::canonicalize(Path::new(&root)).unwrap().to_str().unwrap(),
                record.root_path);
            fs::write(report.with_extension("ready"), b"ready").unwrap();
            registry.enroll(&record).unwrap();
            fs::write(&report, b"enrolled").unwrap();
            return;
        }
        let registry = SharedHostCustodyRegistry::open_existing(Path::new(&db)).unwrap();
        let value = registry.inspect(
            Path::new(&root),
            PhysicalObjectId { device: 1001, file: 9001 },
        ).unwrap().expect("second process cannot lose host registration");
        assert_eq!(value.profile_id, "approved-s3");
        assert!(value.lock_identity.valid());
    }

    #[test]
    fn actual_separate_process_reopens_shared_enrollment() {
        const CHILD: &str =
            "sync::host_custody_registry::tests::host_custody_process_child_entrypoint";
        let (_temp, db, vault, appdata, record) = specimen();
        let mut owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        owner.enroll(&record).unwrap();
        drop(owner);
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(CHILD)
            .arg("--nocapture")
            .env("AKULA_HOST_CUSTODY_CHILD_DB", &db)
            .env("AKULA_HOST_CUSTODY_CHILD_ROOT", &record.root_path)
            .output()
            .unwrap();
        assert!(output.status.success(),
            "real child failed to reopen shared ledger: stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains(
            "host_custody_process_child_entrypoint ... ok"
        ), "child did not actually run the expected test: {:?}", output.status);
    }

    /// Real child OS process and one installer-owned SQLite writer epoch:
    /// ordinary primary+history finishes before another process can enroll
    /// this physical root. Enrollment subsequently forbids the operation.
    /// The ordinary Core entrypoint does not yet acquire this lease itself.
    #[test]
    fn cross_process_enrollment_waits_for_host_epoch_and_denies_later_write() {
        use crate::app_core::{Core, CoreEvent};
        use crate::files::gate;
        use crate::history::Source;
        use crate::sync::filegate_lock_probe::PinnedProbeRoot;
        use std::cell::Cell;
        use std::process::Command;
        use std::sync::Arc;
        use std::thread;
        use std::time::{Duration, Instant};

        const CHILD: &str =
            "sync::host_custody_registry::tests::host_custody_process_child_entrypoint";
        let (temp, db, vault, appdata, mut record) = specimen();
        let observed = PinnedProbeRoot::open(&vault).unwrap().identity();
        record.root_identity = PhysicalObjectId {
            device: observed.0, file: observed.1,
        };
        let mut host = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        let core_b = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core_b.ui_state.resolve_workspace(vault.to_str().unwrap(), 1).unwrap();
        let target = vault.join("PriorOrdinary.md");
        let report = temp.path().join("enrollment-finished.txt");

        let mut child = host.with_unenrolled_epoch(&vault, record.root_identity, || {
            let child = Command::new(std::env::current_exe().unwrap())
                .arg("--exact").arg(CHILD).arg("--nocapture")
                .env("AKULA_HOST_CUSTODY_CHILD_DB", &db)
                .env("AKULA_HOST_CUSTODY_CHILD_ROOT", &record.root_path)
                .env("AKULA_HOST_CUSTODY_CHILD_ENROLL_JSON",
                     serde_json::to_string(&record).unwrap())
                .env("AKULA_HOST_CUSTODY_CHILD_ENROLL_REPORT", &report)
                .spawn().unwrap();
            let start = Instant::now();
            while !report.with_extension("ready").exists() {
                assert!(start.elapsed() < Duration::from_secs(15),
                    "separate enrollment process failed to start");
                thread::sleep(Duration::from_millis(10));
            }
            thread::sleep(Duration::from_millis(180));
            assert!(!report.exists(),
                "enrollment committed while ordinary primary/history held host epoch");
            gate::create(&core_b, &target, "prior content", Source::Me, None)
                .unwrap();
            child
        }).unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(fs::read_to_string(&report).unwrap(), "enrolled");

        let attempted = Cell::new(false);
        assert!(matches!(
            host.with_unenrolled_epoch(&vault, record.root_identity, || {
                attempted.set(true);
                gate::write(&core_b, &target, "forbidden after enrollment",
                    None, Source::Me, None)
            }),
            Err(HostCustodyError::ConflictingEnrollment)
        ));
        assert!(!attempted.get(), "epoch ran a mutation after enrollment");
        assert_eq!(fs::read_to_string(&target).unwrap(), "prior content");
        assert_eq!(host.inspect(&vault, record.root_identity).unwrap(), Some(record));
    }

    #[test]
    fn enrollment_refuses_nested_roots_in_either_order_but_allows_siblings() {
        let (temp, db, vault, appdata, parent) = specimen();
        let nested = vault.join("second-appdata-nested");
        let sibling = temp.path().join("unrelated-root");
        fs::create_dir(&nested).unwrap();
        fs::create_dir(&sibling).unwrap();
        let mut nested_record = parent.clone();
        nested_record.root_path =
            fs::canonicalize(&nested).unwrap().to_string_lossy().into_owned();
        nested_record.root_identity.file += 1;
        nested_record.vault_id = VaultId::new();

        let mut sibling_record = parent.clone();
        sibling_record.root_path =
            fs::canonicalize(&sibling).unwrap().to_string_lossy().into_owned();
        sibling_record.root_identity.file += 2;
        sibling_record.vault_id = VaultId::new();

        let mut host = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        host.enroll(&parent).unwrap();
        assert!(matches!(host.enroll(&nested_record),
            Err(HostCustodyError::ConflictingEnrollment)));
        // A rejected nested insert must not poison the transaction or
        // prohibit a genuinely distinct physical workspace.
        host.enroll(&sibling_record).unwrap();
        assert_eq!(host.inspect(Path::new(&sibling_record.root_path),
            sibling_record.root_identity).unwrap(), Some(sibling_record));
        assert_eq!(host.inspect(Path::new(&parent.root_path),
            parent.root_identity).unwrap(), Some(parent.clone()));
        drop(host);

        // Reverse the order in a separate installer-provisioned database.
        let reverse_db = db.with_file_name("reverse.sqlite");
        let mut host = SharedHostCustodyRegistry::provision_new(
            &reverse_db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        host.enroll(&nested_record).unwrap();
        assert!(matches!(host.enroll(&parent),
            Err(HostCustodyError::ConflictingEnrollment)));
        assert_eq!(host.inspect(Path::new(&nested_record.root_path),
            nested_record.root_identity).unwrap(), Some(nested_record));
    }

    #[test]
    fn nested_workspaces_cannot_evade_an_enrolled_parent_under_host_epoch() {
        let (_temp, db, vault, appdata, record) = specimen();
        let child = vault.join("independent-appdata-child");
        fs::create_dir(&child).unwrap();
        let mut host = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        host.enroll(&record).unwrap();
        let mut changed = false;
        let child_identity = PhysicalObjectId { device: 9000, file: 9999 };
        assert!(matches!(host.with_unenrolled_epoch(
            &child, child_identity, || { changed = true; },
        ), Err(HostCustodyError::ConflictingEnrollment)));
        assert!(!changed, "nested Core escaped enrolled parent");
    }

    #[test]
    fn same_path_new_physical_root_or_same_root_new_vault_is_hold() {
        let (_temp, db, vault, appdata, record) = specimen();
        let mut owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        owner.enroll(&record).unwrap();
        assert!(matches!(owner.inspect(Path::new(&record.root_path),
            PhysicalObjectId { device: 1001, file: 9002 }),
            Err(HostCustodyError::ConflictingEnrollment)));
        let mut foreign = record.clone();
        foreign.vault_id = VaultId::new();
        assert!(matches!(owner.enroll(&foreign),
            Err(HostCustodyError::ConflictingEnrollment)));
        let mut moved = record;
        // Keep a valid absolute path on Windows as well as Unix; a Unix-only
        // slash path is not a valid Windows absolute enrollment candidate.
        moved.root_path = Path::new(&moved.root_path)
            .with_file_name("other-vault").to_string_lossy().into_owned();
        assert!(matches!(owner.enroll(&moved),
            Err(HostCustodyError::ConflictingEnrollment)));
    }

    #[test]
    fn real_physical_root_replacement_refuses_an_unchanged_path() {
        use crate::sync::filegate_lock_probe::PinnedProbeRoot;
        let (temp, db, vault, appdata, mut record) = specimen();
        let observed = PinnedProbeRoot::open(&vault).unwrap().identity();
        record.root_identity = PhysicalObjectId {
            device: observed.0, file: observed.1,
        };
        let mut owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        owner.enroll(&record).unwrap();
        drop(owner);
        let old_path = temp.path().join("vault-moved-away");
        fs::rename(&vault, &old_path).unwrap();
        fs::create_dir(&vault).unwrap();
        let new_id = PinnedProbeRoot::open(&vault).unwrap().identity();
        assert_ne!(observed, new_id, "fixture must replace the physical root");
        let reopened = SharedHostCustodyRegistry::open_existing(&db).unwrap();
        assert!(matches!(reopened.inspect(
            &vault,
            PhysicalObjectId { device: new_id.0, file: new_id.1 },
        ), Err(HostCustodyError::ConflictingEnrollment)));
    }

    #[test]
    fn damaged_index_cannot_disappear_into_unregistered_state() {
        let (_temp, db, vault, appdata, record) = specimen();
        let mut owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        owner.enroll(&record).unwrap();
        drop(owner);
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE shared_custody SET root_key = '0000000000009999:0000000000008888'",
            [],
        ).unwrap();
        drop(conn);
        assert!(matches!(SharedHostCustodyRegistry::open_existing(&db),
            Err(HostCustodyError::DamagedRegistry)));
    }

    #[test]
    fn tamper_checksum_missing_table_and_future_schema_fail_closed() {
        let (_temp, db, vault, appdata, record) = specimen();
        let mut owner = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        owner.enroll(&record).unwrap();
        drop(owner);
        let conn = Connection::open(&db).unwrap();
        conn.execute("UPDATE shared_custody SET record_blake3 = 'fake'", []).unwrap();
        drop(conn);
        // open_existing already walks and validates *every* durable row.
        // An invalid digest must fail at open, before inspect is possible.
        assert!(matches!(SharedHostCustodyRegistry::open_existing(&db),
            Err(HostCustodyError::DamagedRegistry)));
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("PRAGMA user_version = 9;").unwrap();
        drop(conn);
        assert!(matches!(SharedHostCustodyRegistry::open_existing(&db),
            Err(HostCustodyError::UnsupportedSchema)));
    }

    #[test]
    fn caller_spelling_must_not_hide_a_registered_physical_root() {
        let (_temp, db, vault, appdata, record) = specimen();
        let mut installer = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        installer.enroll(&record).unwrap();
        drop(installer);
        let other_core = SharedHostCustodyRegistry::open_existing(&db).unwrap();
        // Pass the uncanonicalized path used by the second Core. On Windows
        // the durable record instead carries an extended-length path.
        assert_eq!(other_core.inspect(&vault, record.root_identity).unwrap(),
            Some(record.clone()));
        let changed_id = PhysicalObjectId {
            file: record.root_identity.file + 1, ..record.root_identity
        };
        assert!(matches!(other_core.inspect(&vault, changed_id),
            Err(HostCustodyError::ConflictingEnrollment)),
            "same physical root pathname with different OS ID must HOLD");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_alias_cannot_represent_same_physical_root_in_host_ledger() {
        let (temp, db, vault, appdata, record) = specimen();
        let mut installer = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        installer.enroll(&record).unwrap();
        drop(installer);
        let alias = temp.path().join("aliased-vault");
        std::os::unix::fs::symlink(&vault, &alias).unwrap();
        let reader = SharedHostCustodyRegistry::open_existing(&db).unwrap();
        assert!(matches!(reader.inspect(&alias, record.root_identity),
            Err(HostCustodyError::InvalidRecord)));
        assert_eq!(reader.inspect(&vault, record.root_identity).unwrap(), Some(record));
    }

    #[cfg(unix)]
    #[test]
    fn substituted_host_parent_symlink_cannot_select_a_private_empty_ledger() {
        let (temp, db, vault, appdata, record) = specimen();
        let mut installer = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        installer.enroll(&record).unwrap();
        drop(installer);

        let old_parent = db.parent().unwrap().to_path_buf();
        let preserved_parent = temp.path().join("preserved-host");
        fs::rename(&old_parent, &preserved_parent).unwrap();

        // An attacker places a syntactically valid, empty registry in another
        // directory and swaps the compiler-pinned parent name to a symlink.
        let private_parent = temp.path().join("private-host");
        fs::create_dir(&private_parent).unwrap();
        let private_db = private_parent.join("approved.sqlite");
        drop(SharedHostCustodyRegistry::provision_new(
            &private_db, &[vault.as_path(), appdata.as_path()],
        ).unwrap());
        std::os::unix::fs::symlink(&private_parent, &old_parent).unwrap();

        assert!(matches!(
            SharedHostCustodyRegistry::open_existing(&db),
            Err(HostCustodyError::InvalidHostLocation),
        ), "parent symlink must HOLD instead of reading an empty foreign ledger");

        // Prove enrollment was not erased, migrated or reset by the refusal.
        let preserved_db = preserved_parent.join("approved.sqlite");
        let original = SharedHostCustodyRegistry::open_existing(&preserved_db).unwrap();
        assert_eq!(original.inspect(&vault, record.root_identity).unwrap(), Some(record));
    }

    #[test]
    fn missing_shared_table_does_not_rebootstrap_a_schema_one_registry() {
        let (_temp, db, vault, appdata, _) = specimen();
        let installer = SharedHostCustodyRegistry::provision_new(
            &db, &[vault.as_path(), appdata.as_path()],
        ).unwrap();
        drop(installer);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("DROP TABLE shared_custody; PRAGMA user_version = 1;").unwrap();
        drop(conn);
        assert!(SharedHostCustodyRegistry::open_existing(&db).is_err());
        assert!(db.exists(), "broken host ledger must never be silently removed");
    }
}
