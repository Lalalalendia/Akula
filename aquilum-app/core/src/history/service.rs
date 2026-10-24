use super::milestone::{self, Before, Incoming, MilestonePlan, Recorded, Source};
use super::recovery;
use super::store;
use crate::files::document::read_text;
use crate::files::error::FileCommandError;
use crate::files::relocate::{move_folder, remove_empty_ancestors, Placement};
use crate::files::trash::trash_path;
use crate::search::paths::{identity, same_path, strip_root};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

const DEVICE_FILE: &str = "device-id";

// Separate from Core File Gate. All HistoryService instances in one process
// serialize version-store reads and writes with native history maintenance.
// No code holding this mutex may acquire Core File Gate, emit host callbacks,
// or perform editor/DocumentHub reconciliation.
#[derive(Default)]
struct HistoryFilesShared {
    // A failed rename can leave version files in both trees. Block reads and
    // writes in every cooperating Core until the SAME transfer is retried.
    // This is in-memory, not a crash-durable recovery journal.
    blocked: HashMap<String, HistoryMoveBlock>,
}

#[derive(Clone)]
struct HistoryMoveBlock {
    from: String,
    to: String,
    reason: String,
    affected_roots: Vec<PathBuf>,
}

static PROCESS_HISTORY_FILES: LazyLock<Mutex<HistoryFilesShared>> =
    LazyLock::new(|| Mutex::new(HistoryFilesShared::default()));

pub struct HistoryService {
    device: String,
    state: Mutex<State>,
    /// Set once by Core::open before exposing any direct HistoryService API.
    /// Standalone HistoryService in legacy tests has no enrolled Core policy.
    guarded_participant: OnceLock<Arc<AtomicBool>>,
    // Once a Core is host-bound, direct history mutation without its held
    // shared enrollment epoch is forbidden, even in fresh private appdata.
    #[cfg(feature = "host-custody")]
    installed_host_requires_epoch: AtomicBool,
    #[cfg(test)]
    next_fault: Mutex<Option<HistoryFault>>,
}

/// A failed history operation is distinct from a successful primary note write.
#[derive(Debug)]
pub struct HistorySaveFailure {
    pub action: &'static str,
    pub message: String,
}

impl HistorySaveFailure {
    fn new(action: &'static str, error: impl fmt::Display) -> Self {
        Self { action, message: error.to_string() }
    }
}

impl fmt::Display for HistorySaveFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.action, self.message)
    }
}

/// P0g-3b: point-in-time version observation verified while holding
/// PROCESS_HISTORY_FILES. Normal retention, reversion and direct same-user
/// edits can later remove/alter these bytes. This is NOT a persistent immutable
/// archive, an OS publication receipt or remote baseline authority.
/// Deliberately non-Clone and sealed: another crate module may inspect
/// this bounded observation but cannot manufacture a "verified" version.
#[must_use = "a verified version is only evidence at the time of inspection"]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedHistoryVersion {
    version_id: String,
    content_hash: String,
    content_length: usize,
}

#[allow(dead_code)]
impl VerifiedHistoryVersion {
    pub(crate) fn version_id(&self) -> &str {
        &self.version_id
    }

    pub(crate) fn content_hash(&self) -> &str {
        &self.content_hash
    }

    pub(crate) fn content_length(&self) -> usize {
        self.content_length
    }
}

/// Read-only cold-start observation for one registered note/version.
/// Deliberately not authority to replace any primary file or to acknowledge S3.
#[must_use = "a cold history observation is forensic only"]
#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct InertColdHistoryNote {
    registered_root: PathBuf,
    note: PathBuf,
    version_id: String,
    content_hash: String,
    content_length: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HistorySaveEffect {
    NoChange,
    LabelOnly,
    Reverted,
    Updated,
    Created(String),
}

impl HistorySaveEffect {
    fn changed(&self) -> bool {
        !matches!(self, Self::NoChange)
    }

    fn candidate_version(&self) -> Option<&str> {
        match self {
            Self::Created(id) => Some(id),
            _ => None,
        }
    }
}

struct HistorySaveOutcome {
    effect: HistorySaveEffect,
    verified_version: Option<VerifiedHistoryVersion>,
}

impl HistorySaveOutcome {
    fn unchanged() -> Self {
        Self { effect: HistorySaveEffect::NoChange, verified_version: None }
    }

    fn changed(&self) -> bool {
        self.effect.changed()
    }
}

/// A primary note relocation already succeeded, but moving its history
/// did not. A partially moved history must not be announced as complete.
#[derive(Debug)]
pub struct HistoryMoveFailure {
    pub action: &'static str,
    pub message: String,
    pub may_have_partial_files: bool,
}

impl HistoryMoveFailure {
    fn new(action: &'static str, error: impl fmt::Display, may_have_partial_files: bool) -> Self {
        Self { action, message: error.to_string(), may_have_partial_files }
    }
}

impl fmt::Display for HistoryMoveFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.action, self.message)
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryFault {
    CreateVersion,
    UpdateVersion,
    NameVersion,
    RemoveVersion,
    MoveBefore,
    MoveAfterOneFile,
}

#[derive(Default)]
struct State {
    roots: Vec<PathBuf>,
    notes: HashMap<String, Note>,
}

// Always acquire PROCESS_HISTORY_FILES before the per-service state mutex.
// disk_before intentionally uses the per-service mutex alone, so a File Gate
// write does not wait on a potentially long history-retention scan.
struct HistoryFilesStateGuard<'a> {
    process: MutexGuard<'static, HistoryFilesShared>,
    state: MutexGuard<'a, State>,
}

impl HistoryFilesStateGuard<'_> {
    fn is_blocked(&self, key: &str) -> bool {
        self.process.blocked.contains_key(key)
    }

    fn assert_retry_pair(&self, from: &str, to: &str, action: &'static str) -> Result<(), HistoryMoveFailure> {
        for key in [from, to] {
            if let Some(block) = self.process.blocked.get(key) {
                if block.from != from || block.to != to {
                    return Err(HistoryMoveFailure::new(
                        action,
                        format!("history transfer still pending from {} to {}: {}", block.from, block.to, block.reason),
                        true,
                    ));
                }
            }
        }
        Ok(())
    }

    fn quarantine(&mut self, from: &str, to: &str, affected_roots: &[&Path], why: &str) {
        let block = HistoryMoveBlock {
            from: from.to_owned(),
            to: to.to_owned(),
            reason: why.to_owned(),
            affected_roots: affected_roots.iter().map(|path| path.to_path_buf()).collect(),
        };
        self.process.blocked.insert(from.to_owned(), block.clone());
        self.process.blocked.insert(to.to_owned(), block);
    }

    fn release_pair(&mut self, from: &str, to: &str) {
        self.process.blocked.remove(from);
        self.process.blocked.remove(to);
    }
}

impl Deref for HistoryFilesStateGuard<'_> {
    type Target = State;

    fn deref(&self) -> &State {
        &self.state
    }
}

impl DerefMut for HistoryFilesStateGuard<'_> {
    fn deref_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

struct Note {
    dir: PathBuf,
    written_hash: Option<String>,
    last_edit: Option<SystemTime>,
    // On first-write failure, retain the pre-commit original. A later note
    // save cannot re-read it after the primary document has been replaced.
    pending_before: Option<(String, SystemTime)>,
}

impl Note {
    fn open(dir: PathBuf) -> Self {
        Self {
            dir,
            written_hash: None,
            last_edit: None,
            pending_before: None,
        }
    }
}

struct Located {
    key: String,
    vault: PathBuf,
    dir: PathBuf,
}

pub type Roots<'a> = &'a dyn Fn() -> Vec<PathBuf>;

pub struct NoteWrite<'a> {
    pub disk_before: Option<(String, SystemTime)>,
    pub text: &'a str,
    pub hash: &'a str,
    pub source: Source,
    pub name: Option<&'a str>,
}

#[derive(Default, Serialize)]
pub struct HistoryPage {
    pub total: usize,
    pub versions: Vec<store::VersionFile>,
}

#[derive(Serialize)]
pub struct VersionTexts {
    pub text: String,
    pub previous: Option<String>,
}

impl HistoryService {
    pub fn new(app_data: &Path) -> Self {
        Self {
            device: device_id(app_data),
            state: Mutex::default(),
            guarded_participant: OnceLock::new(),
            #[cfg(feature = "host-custody")]
            installed_host_requires_epoch: AtomicBool::new(false),
            #[cfg(test)]
            next_fault: Mutex::new(None),
        }
    }

    /// A Core must bind this before workers or UI callbacks can mutate
    /// history. A second bind is a construction error, not a policy reset.
    pub(crate) fn bind_guarded_participant_policy(&self, policy: Arc<AtomicBool>) {
        assert!(self.guarded_participant.set(policy).is_ok(),
            "history participant safety policy already installed");
    }

    #[cfg(feature = "host-custody")]
    pub(crate) fn require_installed_host_epoch(&self) {
        self.installed_host_requires_epoch.store(true, Ordering::Release);
    }

    fn denies_unverified_history_writes(&self) -> bool {
        let enrolled = self.guarded_participant.get()
            .is_some_and(|flag| flag.load(Ordering::Acquire));
        #[cfg(feature = "host-custody")]
        { enrolled || self.installed_host_requires_epoch.load(Ordering::Acquire) }
        #[cfg(not(feature = "host-custody"))]
        { enrolled }
    }

    /// Serialize the transition into participant HOLD with every in-flight
    /// direct HistoryService mutation across all cooperating Core instances.
    /// The caller MUST already own PROCESS_FILE_MUTATIONS and may NOT run
    /// callbacks, enter File Gate again, or execute a normal HistoryService
    /// mutator from inside the closure.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn with_participant_enrollment_fence<T>(
        &self,
        operation: impl FnOnce() -> Result<T, FileCommandError>,
    ) -> Result<T, FileCommandError> {
        let _history_files = PROCESS_HISTORY_FILES.lock()
            .map_err(|_| FileCommandError::Task {
                message: "HistoryService enrollment fence poisoned; refusing participant activation".into(),
            })?;
        operation()
    }

    pub fn disk_before(
        &self,
        note: &Path,
        expected_hash: Option<&str>,
    ) -> Option<(String, SystemTime)> {
        let key = identity(note);
        let known = self.lock().notes.get(&key).is_some_and(|state| {
            expected_hash.is_some() && state.written_hash.as_deref() == expected_hash
        });
        if known {
            return None;
        }
        let text = read_text(note).ok()?;
        let at = fs::metadata(note)
            .and_then(|metadata| metadata.modified())
            .unwrap_or_else(|_| SystemTime::now());
        Some((text, at))
    }

    /// Legacy bool wrapper. File Gate uses try_saved so it can propagate
    /// a history-specific failure without retracting the primary note save.
    pub fn saved(&self, roots: Roots<'_>, note: &Path, write: NoteWrite<'_>) -> bool {
        match self.try_saved(roots, note, write) {
            Ok(changed) => changed,
            Err(error) => {
                report(note, "не сохранена версия", &error.to_string());
                false
            }
        }
    }

    /// Ok(true) requires a persisted history change; Ok(false) means no change
    /// was required. Err means planned history persistence was incomplete.
    pub fn try_saved(
        &self,
        roots: Roots<'_>,
        note: &Path,
        write: NoteWrite<'_>,
    ) -> Result<bool, HistorySaveFailure> {
        if self.denies_unverified_history_writes() {
            return Err(HistorySaveFailure::new(
                "guarded history HOLD",
                "direct history save lacks pinned S3 custody",
            ));
        }
        self.try_saved_impl(roots, note, write, false, false)
            .map(|outcome| outcome.changed())
    }

    /// A host-bound ordinary File Gate note write is admitted only while the
    /// same physical Core holds the installer-wide unenrolled SQLite epoch.
    /// No direct history caller can construct this lexical capability.
    #[cfg(feature = "host-custody")]
    pub(crate) fn try_saved_with_host_epoch(
        &self,
        permit: &crate::app_core::HostUnenrolledEpochPermit<'_>,
        roots: Roots<'_>,
        note: &Path,
        write: NoteWrite<'_>,
    ) -> Result<bool, HistorySaveFailure> {
        if !permit.binds_history(self) {
            return Err(HistorySaveFailure::new("host history epoch", "wrong Core owner"));
        }
        self.try_saved_impl(roots, note, write, true, false)
            .map(|outcome| outcome.changed())
    }

    /// Narrow checked File Gate exception. The sealed token can be created
    /// only by the sync wrapper, while inside its physically held OS lock.
    /// No direct Core/host caller can construct that token.
    pub(crate) fn try_saved_with_custody(
        &self,
        permit: &crate::sync::guarded_note_write::HeldCustodyHistoryPermit<'_>,
        roots: Roots<'_>,
        note: &Path,
        write: NoteWrite<'_>,
    ) -> Result<bool, HistorySaveFailure> {
        if !permit.binds_history_and_note(self, note) {
            return Err(HistorySaveFailure::new(
                "custody history owner", "wrong Core/HistoryService or note",
            ));
        }
        self.try_saved_impl(roots, note, write, true, false)
            .map(|outcome| outcome.changed())
    }

    /// Strict internal history-version proof for a future protected receipt.
    /// The sealed custody token must originate from the guarded File Gate.
    /// No proof is returned for quiet, label-only, revert, coalesced update,
    /// missing history location or failed byte-for-byte readback.
    /// A coalesced latest version is MUTABLE and cannot serve as an immutable
    /// history identity for a future native publication receipt.
    /// This does not provide the OS file/directory durability evidence.
    #[allow(dead_code)]
    pub(crate) fn try_saved_with_custody_verified_version(
        &self,
        permit: &crate::sync::guarded_note_write::HeldCustodyHistoryPermit<'_>,
        roots: Roots<'_>,
        note: &Path,
        write: NoteWrite<'_>,
    ) -> Result<VerifiedHistoryVersion, HistorySaveFailure> {
        if !permit.binds_history_and_note(self, note) {
            return Err(HistorySaveFailure::new(
                "custody history owner", "wrong Core/HistoryService or note",
            ));
        }
        self.try_saved_impl(roots, note, write, true, true)?
            .verified_version
            .ok_or_else(|| HistorySaveFailure::new(
                "history version proof", "snapshot verification was unavailable",
            ))
    }

    fn try_saved_impl(
        &self,
        roots: Roots<'_>,
        note: &Path,
        write: NoteWrite<'_>,
        custody_verified: bool,
        require_verified_version: bool,
    ) -> Result<HistorySaveOutcome, HistorySaveFailure> {
        #[cfg(feature = "save-stage-probe")]
        let profile_start = std::time::Instant::now();
        let now = SystemTime::now();
        let Some(mut state) = self.files_lock() else {
            return Err(HistorySaveFailure::new("history lock", "poisoned"));
        };
        #[cfg(feature = "save-stage-probe")]
        let profile_locked = profile_start.elapsed();
        // Check again AFTER PROCESS_HISTORY_FILES: activation takes that same
        // mutex while publishing its durable HOLD. A direct writer that saw
        // a false flag before waiting cannot publish history after enrollment.
        if !custody_verified && self.denies_unverified_history_writes() {
            return Err(HistorySaveFailure::new(
                "guarded history HOLD",
                "participant enrollment occurred before history mutation",
            ));
        }
        let Some(located) = locate(&mut state, roots, note) else {
            return if require_verified_version {
                Err(HistorySaveFailure::new(
                    "history version proof", "registered note has no history location",
                ))
            } else {
                Ok(HistorySaveOutcome::unchanged())
            };
        };
        if history_blocked(&state, &located) {
            return Err(HistorySaveFailure::new("history relocation pending", "pending or unreadable version-transfer intent; history write refused"));
        }
        if !allowed_history_directory(&located.vault, &located.dir) {
            return Err(HistorySaveFailure::new("history path", "unsafe history directory"));
        }
        store::ensure_version_integrity(&located.dir).map_err(|error|
            HistorySaveFailure::new("history integrity migration", error)
        )?;
        let entry = state
            .notes
            .entry(located.key)
            .or_insert_with(|| Note::open(located.dir.clone()));

        let versions = store::versions(&located.dir);
        let names = store::names(&located.dir);
        let latest_text = versions.last().and_then(|v| store::read_version(&located.dir, v));
        let previous_text = (versions.len() >= 2)
            .then(|| store::read_version(&located.dir, &versions[versions.len() - 2]))
            .flatten();
        #[cfg(feature = "save-stage-probe")]
        let profile_loaded = profile_start.elapsed();
        let disk = entry
            .pending_before
            .as_ref()
            .or(write.disk_before.as_ref())
            .map(|(text, at)| Before { text, at: *at });

        let plan = milestone::plan(
            &self.device,
            &Incoming {
                source: write.source,
                text: write.text,
                disk_before: disk.as_ref(),
            },
            &Recorded {
                versions: &versions,
                names: &names,
                latest_text: latest_text.as_deref(),
                previous_text: previous_text.as_deref(),
                last_edit: entry.last_edit,
            },
            now,
        );
        #[cfg(feature = "save-stage-probe")]
        let profile_planned = profile_start.elapsed();
        // A strict receipt request must not mutate history then announce
        // refusal. Quiet, label-only, revert and mutable latest-version
        // coalescing are rejected BEFORE *any* history-store filesystem write.
        // Existing non-strict editor/host APIs retain their legacy semantics.
        if require_verified_version {
            const MAX_RECEIPT_HISTORY_BYTES: usize = 32 * 1024 * 1024;
            // A Source::Me snapshot could later be coalesced in-place by
            // the ordinary editor, invalidating a claimed immutable ID.
            // The S3 path is external by definition and must never mint
            // history evidence through a mutable local-typing milestone.
            if write.text.len() > MAX_RECEIPT_HISTORY_BYTES
                || write.source != Source::External
                || !matches!(&plan, MilestonePlan::CreateNew { .. })
            {
                return Err(HistorySaveFailure::new(
                    "history version proof",
                    "strict evidence requires bounded external CreateNew snapshot",
                ));
            }
        }
        let quiet = matches!(&plan, MilestonePlan::Quiet);
        let dir = &located.dir;
        let outcome = (|| -> Result<HistorySaveEffect, HistorySaveFailure> {
            match plan {
                MilestonePlan::Quiet => Ok(HistorySaveEffect::NoChange),
                MilestonePlan::Skip => match (write.name, versions.last()) {
                    (Some(name), Some(latest)) => {
                        self.write_name(dir, &latest.id, name, now)
                            .map_err(|error| HistorySaveFailure::new("write version name", error))?;
                        Ok(HistorySaveEffect::LabelOnly)
                    }
                    _ => Ok(HistorySaveEffect::NoChange),
                },
                MilestonePlan::RevertToPrevious { remove_id } => {
                    let removed = self.remove_version_and_names(dir, &remove_id);
                    // A later label deletion may fail after unlinking the
                    // version. Keep coalescing state aligned with the disk.
                    if !dir.join(&remove_id).exists() {
                        entry.last_edit = None;
                    }
                    removed.map_err(|error| HistorySaveFailure::new("remove version", error))?;
                    Ok(HistorySaveEffect::Reverted)
                }
                MilestonePlan::UpdateLatest { id } => {
                    self.update_version(dir, &id, write.text)
                        .map_err(|error| HistorySaveFailure::new("update version", error))?;
                    entry.last_edit = Some(now);
                    if let Some(name) = write.name {
                        self.write_name(dir, &id, name, now)
                            .map_err(|error| HistorySaveFailure::new("write version name", error))?;
                    }
                    Ok(HistorySaveEffect::Updated)
                }
                MilestonePlan::CreateNew { baseline, snapshot } => {
                    if let Some(baseline) = baseline {
                        self.write_version(dir, &baseline)
                            .map_err(|error| HistorySaveFailure::new("write baseline", error))?;
                    }
                    let path = self
                        .write_version(dir, &snapshot)
                        .map_err(|error| HistorySaveFailure::new("write version", error))?;
                    entry.last_edit = Some(now);
                    if let (Some(name), Some(id)) = (
                        write.name,
                        path.file_name().and_then(|file| file.to_str()),
                    ) {
                        self.write_name(dir, id, name, now)
                            .map_err(|error| HistorySaveFailure::new("write version name", error))?;
                    }
                    let id = path.file_name()
                        .and_then(|component| component.to_str())
                        .ok_or_else(|| HistorySaveFailure::new(
                            "history version identity", "written history ID not UTF-8",
                        ))?.to_owned();
                    Ok(HistorySaveEffect::Created(id))
                }
            }
        })();
        #[cfg(feature = "save-stage-probe")]
        {
            let profile_done = profile_start.elapsed();
            let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
            eprintln!(
                "AKULA_SAVE_HISTORY lock_ms={:.4} inspect_ms={:.4} plan_ms={:.4} persistence_ms={:.4} total_ms={:.4} versions={} ok={}",
                ms(profile_locked),
                ms(profile_loaded - profile_locked),
                ms(profile_planned - profile_loaded),
                ms(profile_done - profile_planned),
                ms(profile_done),
                versions.len(),
                u8::from(outcome.is_ok()),
            );
        }
        match outcome {
            Ok(effect) => {
                // This check stays INSIDE PROCESS_HISTORY_FILES and the
                // HistoryService state lock, never a later page() observation.
                // Re-read the exact version created/updated by this call.
                let verified_version = if require_verified_version {
                    let id = effect.candidate_version().ok_or_else(|| {
                        HistorySaveFailure::new(
                            "history version proof",
                            "no new or updated snapshot to attest",
                        )
                    })?;
                    Some(Self::verify_history_version(
                        dir, id, write.text, write.hash,
                    )?)
                } else {
                    None
                };
                // A quiet reader update must not discard a failed baseline
                // awaiting the next real save.
                if !quiet || entry.pending_before.is_none() {
                    entry.written_hash = Some(write.hash.to_owned());
                    entry.pending_before = None;
                }
                Ok(HistorySaveOutcome { effect, verified_version })
            }
            Err(error) => {
                if versions.is_empty() && entry.pending_before.is_none() {
                    entry.pending_before = write.disk_before;
                }
                Err(error)
            }
        }
    }

    /// Read back only the precise file ID returned by this mutation. The
    /// ordinary history API deliberately skips this expensive validation.
    /// It is not proof against hostile same-UID substitution after unlock.
    fn verify_history_version(
        dir: &Path,
        id: &str,
        expected: &str,
        expected_hash: &str,
    ) -> Result<VerifiedHistoryVersion, HistorySaveFailure> {
        use std::io::Read;
        const MAX_RECEIPT_HISTORY_BYTES: u64 = 32 * 1024 * 1024;
        let path = dir.join(id);
        let named = fs::symlink_metadata(&path).map_err(|e|
            HistorySaveFailure::new("history metadata", e)
        )?;
        if !named.is_file() || named.file_type().is_symlink() {
            return Err(HistorySaveFailure::new(
                "history version proof", "history version is an alias or non-file",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if named.nlink() != 1 {
                return Err(HistorySaveFailure::new(
                    "history version proof", "hardlinked history version",
                ));
            }
        }
        let mut file = fs::File::open(&path).map_err(|e|
            HistorySaveFailure::new("history readback", e)
        )?;
        let before = file.metadata().map_err(|e|
            HistorySaveFailure::new("history metadata", e)
        )?;
        if !before.is_file()
            || before.len() > MAX_RECEIPT_HISTORY_BYTES
            || before.len() != expected.len() as u64
        {
            return Err(HistorySaveFailure::new(
                "history version proof", "invalid history size or file type",
            ));
        }
        // Bound the READ itself, not just the initial metadata. Another
        // process may grow this pathname while it is being consumed.
        let mut bytes = Vec::with_capacity(expected.len());
        (&mut file).take(MAX_RECEIPT_HISTORY_BYTES + 1)
            .read_to_end(&mut bytes).map_err(|e|
                HistorySaveFailure::new("history readback", e)
            )?;
        let after = file.metadata().map_err(|e|
            HistorySaveFailure::new("history metadata", e)
        )?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        if bytes.len() as u64 > MAX_RECEIPT_HISTORY_BYTES
            || bytes != expected.as_bytes()
            || digest != expected_hash
            || before.len() != after.len()
            || bytes.len() != expected.len()
        {
            return Err(HistorySaveFailure::new(
                "history version proof", "history contents differ after persistence",
            ));
        }
        if !store::verify_version_digest(dir, id, &digest).map_err(|error|
            HistorySaveFailure::new("history BLAKE3 metadata", error)
        )? {
            return Err(HistorySaveFailure::new(
                "history version proof", "persistent BLAKE3 metadata missing or mismatched",
            ));
        }
        Ok(VerifiedHistoryVersion {
            version_id: id.to_owned(),
            content_hash: digest,
            content_length: bytes.len(),
        })
    }

    /// P0g-3d-05: reconstruct a *non-authoritative* exact external version
    /// observation from the registered note after HistoryService reopens.
    ///
    /// This is a bounded read with the process HistoryService mutex held.
    /// It cannot grant a File Gate write, S3 apply/ACK, V8 receipt promotion
    /// or ConfirmedBaseline advancement. Nor does it prove a power-cut-safe
    /// directory entry or exclude hostile same-UID namespace replacement.
    #[allow(dead_code)]
    pub(crate) fn inspect_cold_external_version(
        &self,
        roots: Roots<'_>,
        note: &Path,
        id: &str,
        expected_hash: &str,
        expected_length: u64,
    ) -> Result<InertColdHistoryNote, HistorySaveFailure> {
        use std::io::Read;
        const LIMIT: u64 = 32 * 1024 * 1024;
        let good_hash = expected_hash.len() == 64
            && expected_hash.bytes().all(|b|
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !good_hash || expected_length > LIMIT || id.is_empty() {
            return Err(HistorySaveFailure::new(
                "cold history proof", "invalid immutable version bounds",
            ));
        }

        let Some(mut state) = self.files_lock() else {
            return Err(HistorySaveFailure::new(
                "cold history lock", "history mutex poisoned",
            ));
        };
        // Cold-read authority uses CURRENT host registration, not previously
        // cached roots retained by an earlier call to locate().
        let active_roots=roots();
        if active_roots.iter().filter(|root|
            relative_inside(root,note).is_some()
        ).count()!=1 {
            return Err(HistorySaveFailure::new(
                "cold history location", "note not in exactly one active root",
            ));
        }
        state.roots=active_roots.clone();
        let located = locate(&mut state, &||active_roots.clone(), note)
            .ok_or_else(||HistorySaveFailure::new(
                "cold history location", "note outside registered roots",
            ))?;
        if history_blocked(&state, &located)
            || !allowed_history_directory(&located.vault, &located.dir)
        {
            return Err(HistorySaveFailure::new(
                "cold history location", "unsafe or blocked history directory",
            ));
        }
        store::ensure_version_integrity(&located.dir).map_err(|error|
            HistorySaveFailure::new("cold history integrity migration", error)
        )?;
        // Filename and Source::External must BOTH originate from the parser
        // for THIS note's history directory, not from untrusted receipt text.
        let listed = store::versions(&located.dir).into_iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| HistorySaveFailure::new(
                "cold history version", "version ID absent for registered note",
            ))?;
        if listed.source != Source::External {
            return Err(HistorySaveFailure::new(
                "cold history version", "version was not an external CreateNew",
            ));
        }
        let name = located.dir.join(&listed.id);
        let named = fs::symlink_metadata(&name).map_err(|e|
            HistorySaveFailure::new("cold history metadata", e)
        )?;
        if !named.is_file() || named.file_type().is_symlink() {
            return Err(HistorySaveFailure::new(
                "cold history file", "version is not a plain regular file",
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if named.file_attributes() & 0x0000_0400 != 0 {
                return Err(HistorySaveFailure::new(
                    "cold history file", "Windows reparse point refused",
                ));
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if named.nlink() != 1 {
                return Err(HistorySaveFailure::new(
                    "cold history file", "hardlinked history version refused",
                ));
            }
        }
        let mut file = fs::File::open(&name).map_err(|e|
            HistorySaveFailure::new("cold history file open", e)
        )?;
        let before = file.metadata().map_err(|e|
            HistorySaveFailure::new("cold history file metadata", e)
        )?;
        if !before.is_file() || before.len() != expected_length {
            return Err(HistorySaveFailure::new(
                "cold history bytes", "wrong file type or length",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (named.dev(), named.ino()) != (before.dev(), before.ino()) {
                return Err(HistorySaveFailure::new(
                    "cold history file", "name and opened handle differ",
                ));
            }
        }
        let mut bytes=Vec::with_capacity(expected_length as usize);
        (&mut file).take(LIMIT+1).read_to_end(&mut bytes).map_err(|e|
            HistorySaveFailure::new("cold history file read", e)
        )?;
        let after = file.metadata().map_err(|e|
            HistorySaveFailure::new("cold history file metadata", e)
        )?;
        if bytes.len() as u64 != expected_length
            || before.len()!=after.len()
            || std::str::from_utf8(&bytes).is_err()
            || blake3::hash(&bytes).to_hex().as_str() != expected_hash
        {
            return Err(HistorySaveFailure::new(
                "cold history bytes", "version bytes or digest changed",
            ));
        }
        if !store::verify_version_digest(&located.dir, id, expected_hash).map_err(|error|
            HistorySaveFailure::new("cold history BLAKE3 metadata", error)
        )? {
            return Err(HistorySaveFailure::new(
                "cold history bytes", "persistent BLAKE3 metadata missing or mismatched",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (before.dev(),before.ino())!=(after.dev(),after.ino()) {
                return Err(HistorySaveFailure::new(
                    "cold history file", "opened handle identity changed",
                ));
            }
        }
        // Bind the readback to the resolved registered location. This is
        // deliberately not the non-Clone VerifiedHistoryVersion write token.
        Ok(InertColdHistoryNote {
            registered_root: located.vault,
            note: note.to_path_buf(),
            version_id: listed.id,
            content_hash: expected_hash.to_owned(),
            content_length: expected_length,
        })
    }

    fn write_version(&self, dir: &Path, snapshot: &milestone::Snapshot<'_>) -> Result<PathBuf, FileCommandError> {
        #[cfg(test)]
        self.fail_if_requested(HistoryFault::CreateVersion)?;
        store::write_version(dir, &self.device, snapshot)
    }

    fn update_version(&self, dir: &Path, id: &str, text: &str) -> Result<(), FileCommandError> {
        #[cfg(test)]
        self.fail_if_requested(HistoryFault::UpdateVersion)?;
        store::update_version_text(dir, id, text)
    }

    fn remove_version_and_names(&self, dir: &Path, id: &str) -> Result<(), FileCommandError> {
        #[cfg(test)]
        self.fail_if_requested(HistoryFault::RemoveVersion)?;
        store::remove_version_and_names(dir, id)
    }

    fn write_name(&self, dir: &Path, id: &str, name: &str, at: SystemTime) -> Result<(), FileCommandError> {
        #[cfg(test)]
        self.fail_if_requested(HistoryFault::NameVersion)?;
        store::write_name(dir, &self.device, id, name, at)
    }

    #[cfg(test)]
    pub(crate) fn fail_once(&self, fault: HistoryFault) {
        *self.next_fault.lock().unwrap() = Some(fault);
    }

    #[cfg(test)]
    fn fail_if_requested(&self, kind: HistoryFault) -> Result<(), FileCommandError> {
        let mut pending = self.next_fault.lock().unwrap();
        if *pending == Some(kind) {
            *pending = None;
            return Err(FileCommandError::Io {
                message: "injected history persistence failure".to_owned(),
            });
        }
        Ok(())
    }

    pub fn name(
        &self,
        roots: Roots<'_>,
        note: &Path,
        id: Option<&str>,
        name: &str,
    ) -> Option<String> {
        if self.denies_unverified_history_writes() { return None; }
        let Some(mut state) = self.files_lock() else {
            return None;
        };
        if self.denies_unverified_history_writes() { return None; }
        let located = locate(&mut state, roots, note)?;
        if history_blocked(&state, &located) || !allowed_history_directory(&located.vault, &located.dir) {
            return None;
        }
        let target = match id {
            Some(id) => Some(
                store::versions(&located.dir)
                    .into_iter()
                    .find(|version| version.id == id)?
                    .id,
            ),
            None => store::versions(&located.dir).last().map(|v| v.id.clone()),
        };
        name_version(&self.device, &located.dir, target, name)
    }

    pub fn page(
        &self,
        roots: Roots<'_>,
        note: &Path,
        offset: usize,
        limit: usize,
    ) -> HistoryPage {
        let Some(mut state) = self.files_lock() else {
            return HistoryPage::default();
        };
        let Some(located) = locate(&mut state, roots, note) else {
            return HistoryPage::default();
        };
        if history_blocked(&state, &located) || !allowed_history_directory(&located.vault, &located.dir) {
            return HistoryPage::default();
        }
        if let Err(error) = store::ensure_version_integrity(&located.dir) {
            eprintln!(
                "[aquilum:history] BLAKE3 migration refused for {}: {error}",
                located.dir.display()
            );
            return HistoryPage::default();
        }
        let current = read_text(note).ok();
        let names = store::names(&located.dir);
        let versions = store::versions(&located.dir);
        HistoryPage {
            total: versions.len(),
            versions: versions
                .into_iter()
                .rev()
                .skip(offset)
                .take(limit)
                .map(|mut version| {
                    version.name = names.get(&version.id).cloned();
                    version.is_current = current
                        .as_deref()
                        .is_some_and(|text| store::has_text(&located.dir, &version, text));
                    version
                })
                .collect(),
        }
    }

    pub fn version(
        &self,
        roots: Roots<'_>,
        note: &Path,
        id: Option<&str>,
    ) -> Option<VersionTexts> {
        let Some(mut state) = self.files_lock() else {
            return None;
        };
        let located = locate(&mut state, roots, note)?;
        if history_blocked(&state, &located) || !allowed_history_directory(&located.vault, &located.dir) {
            return None;
        }
        if let Err(error) = store::ensure_version_integrity(&located.dir) {
            eprintln!(
                "[aquilum:history] BLAKE3 migration refused for {}: {error}",
                located.dir.display()
            );
            return None;
        }
        let versions = store::versions(&located.dir);
        let id = id?;
        let index = versions.iter().position(|version| version.id == id)?;
        let previous = index
            .checked_sub(1)
            .and_then(|previous| store::read_version(&located.dir, &versions[previous]));
        Some(VersionTexts {
            text: store::read_version(&located.dir, &versions[index])?,
            previous,
        })
    }

    /// Legacy void API; use try_* in File Gate to report history failures
    /// separately from an already successful primary file relocation.
    pub fn moved(&self, roots: Roots<'_>, from: &Path, to: &Path) {
        if let Err(error) = self.try_moved(roots, from, to) {
            report(from, "не перенесена", &error.to_string());
        }
    }

    pub fn try_moved(&self, roots: Roots<'_>, from: &Path, to: &Path) -> Result<(), HistoryMoveFailure> {
        let action = "move history";
        if self.denies_unverified_history_writes() {
            return Err(HistoryMoveFailure::new(action,
                "direct history mutation refused by enrolled workspace HOLD", false));
        }
        let Some(mut state) = self.files_lock() else {
            return Err(HistoryMoveFailure::new(action, "history mutex poisoned", false));
        };
        if self.denies_unverified_history_writes() {
            return Err(HistoryMoveFailure::new(action,
                "participant HOLD activated before direct history relocation", false));
        }
        let (Some(source), Some(target)) = (
            locate(&mut state, roots, from),
            locate(&mut state, roots, to),
        ) else {
            return Ok(());
        };
        state.assert_retry_pair(&source.key, &target.key, action)?;
        if !same_path(&source.vault, &target.vault) {
            // The legacy source-only journal cannot protect a second Core
            // that has registered only the destination workspace.
            // Normal File Gate preflight rejects this BEFORE the primary
            // note move; refuse any direct call here without relocating
            // history or claiming a successful transfer.
            state.quarantine(
                &source.key,
                &target.key,
                &[source.vault.as_path(), target.vault.as_path()],
                "cross-vault history transfer requires bilateral admission",
            );
            return Err(HistoryMoveFailure::new(
                action,
                "cross-vault history transfer refused: source-only recovery is unsafe",
                false,
            ));
        }
        if !allowed_history_directory(&source.vault, &source.dir)
            || !allowed_history_directory(&target.vault, &target.dir)
        {
            state.quarantine(&source.key, &target.key, &[source.vault.as_path(), target.vault.as_path()], "unsafe history directory");
            return Err(HistoryMoveFailure::new(action, "unsafe history directory", false));
        }
        if let Err(error) = recovery::transfer(
            &source.vault,
            "move",
            from,
            to,
            &source.vault,
            &target.vault,
            &source.dir,
            &target.dir,
            || self.move_history(&source.dir, &target.dir, Placement::Workspace, &source.vault),
        ) {
            state.quarantine(&source.key, &target.key, &[source.vault.as_path(), target.vault.as_path()], &error.to_string());
            return Err(HistoryMoveFailure::new(action, error, true));
        }
        let note = state.notes.remove(&source.key);
        state.notes.remove(&target.key);
        if let Some(mut note) = note {
            note.dir = target.dir;
            state.notes.insert(target.key.clone(), note);
        }
        state.release_pair(&source.key, &target.key);
        Ok(())
    }

    pub fn trashed(&self, roots: Roots<'_>, note: &Path, trashed_at: &Path) {
        if let Err(error) = self.try_trashed(roots, note, trashed_at) {
            report(note, "не перемещена в корзину", &error.to_string());
        }
    }

    pub fn try_trashed(&self, roots: Roots<'_>, note: &Path, trashed_at: &Path) -> Result<(), HistoryMoveFailure> {
        let action = "trash history";
        if self.denies_unverified_history_writes() {
            return Err(HistoryMoveFailure::new(action,
                "direct history mutation refused by enrolled workspace HOLD", false));
        }
        let Some(mut state) = self.files_lock() else {
            return Err(HistoryMoveFailure::new(action, "history mutex poisoned", false));
        };
        if self.denies_unverified_history_writes() {
            return Err(HistoryMoveFailure::new(action,
                "participant HOLD activated before direct history relocation", false));
        }
        let Some(located) = locate(&mut state, roots, note) else {
            return Ok(());
        };
        let trash = trash_path(&located.vault);
        let Some(in_trash) = relative_inside(&trash, trashed_at) else {
            return Err(HistoryMoveFailure::new(action, "trash destination outside workspace trash root", false));
        };
        let target = store::history_dir(&trash, &in_trash);
        let destination_key = identity(trashed_at);
        state.assert_retry_pair(&located.key, &destination_key, action)?;
        if !allowed_history_directory(&located.vault, &located.dir)
            || !allowed_history_directory(&trash, &target)
        {
            state.quarantine(&located.key, &destination_key, &[located.vault.as_path()], "unsafe history directory");
            return Err(HistoryMoveFailure::new(action, "unsafe history directory", false));
        }
        if let Err(error) = recovery::transfer(
            &located.vault,
            "trash",
            note,
            trashed_at,
            &located.vault,
            &trash,
            &located.dir,
            &target,
            || self.move_history(&located.dir, &target, Placement::Trash, &located.vault),
        ) {
            state.quarantine(&located.key, &destination_key, &[located.vault.as_path()], &error.to_string());
            return Err(HistoryMoveFailure::new(action, error, true));
        }
        let note = state.notes.remove(&located.key);
        if let Some(mut note) = note {
            note.dir = target;
            state.notes.insert(destination_key.clone(), note);
        }
        state.release_pair(&located.key, &destination_key);
        Ok(())
    }

    pub fn restored(&self, roots: Roots<'_>, trashed_at: &Path, restored: &Path) {
        if let Err(error) = self.try_restored(roots, trashed_at, restored) {
            report(restored, "не восстановлена из корзины", &error.to_string());
        }
    }

    pub fn try_restored(&self, roots: Roots<'_>, trashed_at: &Path, restored: &Path) -> Result<(), HistoryMoveFailure> {
        let action = "restore history";
        if self.denies_unverified_history_writes() {
            return Err(HistoryMoveFailure::new(action,
                "direct history mutation refused by enrolled workspace HOLD", false));
        }
        let Some(mut state) = self.files_lock() else {
            return Err(HistoryMoveFailure::new(action, "history mutex poisoned", false));
        };
        if self.denies_unverified_history_writes() {
            return Err(HistoryMoveFailure::new(action,
                "participant HOLD activated before direct history relocation", false));
        }
        let Some(located) = locate(&mut state, roots, restored) else {
            return Ok(());
        };
        let trash = trash_path(&located.vault);
        let Some(in_trash) = relative_inside(&trash, trashed_at) else {
            return Err(HistoryMoveFailure::new(action, "trash source outside workspace trash root", false));
        };
        let source = store::history_dir(&trash, &in_trash);
        let source_key = identity(trashed_at);
        state.assert_retry_pair(&source_key, &located.key, action)?;
        if !allowed_history_directory(&trash, &source)
            || !allowed_history_directory(&located.vault, &located.dir)
        {
            state.quarantine(&source_key, &located.key, &[located.vault.as_path()], "unsafe history directory");
            return Err(HistoryMoveFailure::new(action, "unsafe history directory", false));
        }
        if let Err(error) = recovery::transfer(
            &located.vault,
            "restore",
            trashed_at,
            restored,
            &trash,
            &located.vault,
            &source,
            &located.dir,
            || self.move_history(&source, &located.dir, Placement::Workspace, &trash),
        ) {
            state.quarantine(&source_key, &located.key, &[located.vault.as_path()], &error.to_string());
            return Err(HistoryMoveFailure::new(action, error, true));
        }
        let note = state.notes.remove(&source_key);
        state.notes.remove(&located.key);
        if let Some(mut note) = note {
            note.dir = located.dir;
            state.notes.insert(located.key.clone(), note);
        }
        state.release_pair(&source_key, &located.key);
        Ok(())
    }

    /// A deterministic fault test can interrupt after exactly one version
    /// moved. Failed transfers never advance in-memory note paths.
    fn move_history(&self, from: &Path, to: &Path, placement: Placement, base: &Path) -> Result<(), FileCommandError> {
        #[cfg(test)]
        self.fail_history_relocation_if_requested(from, to)?;
        move_folder(from, to, placement)?;
        remove_empty_ancestors(from, base);
        Ok(())
    }

    #[cfg(test)]
    fn fail_history_relocation_if_requested(&self, from: &Path, to: &Path) -> Result<(), FileCommandError> {
        let mut pending = self.next_fault.lock().unwrap();
        let fault = match *pending {
            Some(HistoryFault::MoveBefore) => Some(HistoryFault::MoveBefore),
            Some(HistoryFault::MoveAfterOneFile) => Some(HistoryFault::MoveAfterOneFile),
            _ => None,
        };
        if fault.is_none() {
            return Ok(());
        }
        *pending = None;
        drop(pending);
        if fault == Some(HistoryFault::MoveAfterOneFile) && from.is_dir() {
            if let Some(entry) = fs::read_dir(from)?.next() {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    fs::create_dir_all(to)?;
                    fs::rename(entry.path(), to.join(entry.file_name()))?;
                }
            }
        }
        Err(FileCommandError::Io {
            message: "injected interrupted history transfer".to_owned(),
        })
    }

    /// Serialize history version-store access across all Core instances.
    /// If the process-wide interlock was poisoned, fail closed instead of
    /// continuing to mutate possibly inconsistent history bytes.
    fn files_lock(&self) -> Option<HistoryFilesStateGuard<'_>> {
        let process = match PROCESS_HISTORY_FILES.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("[aquilum:history] process history-files lock poisoned; access refused");
                return None;
            }
        };
        let state = self.lock();
        Some(HistoryFilesStateGuard { process, state })
    }

    /// Run legacy migration and retention without acquiring Core File Gate.
    /// This takes the history-files mutex only, never the per-service state
    /// mutex; disk_before (which can run under File Gate) stays responsive.
    /// Full-tree housekeeping can delay other history operations but cannot
    /// block the ordinary File Gate mutex directly.
    pub fn maintain_retention(&self, workspace: &Path, retention_days: u32) -> Result<u64, &'static str> {
        if self.denies_unverified_history_writes() {
            return Err("history retention refused by enrolled workspace HOLD");
        }
        let guard = PROCESS_HISTORY_FILES
            .lock()
            .map_err(|_| "process history-files lock poisoned; refusing cleanup")?;
        if self.denies_unverified_history_writes() {
            return Err("history retention denied: participant HOLD activated while waiting for history lock");
        }
        // A failed transfer in one vault must not stall retention in every
        // unrelated vault. The global lock is still held only during work.
        if guard.blocked.values().any(|block| block.affected_roots.iter().any(|root| {
            root.as_path() == workspace || trash_path(root).as_path() == workspace
        })) {
            return Err("history relocation pending in selected workspace; retention deferred");
        }
        let recovery_root = if workspace.file_name().is_some_and(|name| name == ".trash") {
            workspace.parent().unwrap_or(workspace)
        } else {
            workspace
        };
        if !matches!(recovery::pending(recovery_root), Ok(false)) {
            return Err("pending or invalid on-disk history move intent; retention deferred");
        }
        store::adopt_quantum_history(workspace);
        store::adopt_quantum_history(&trash_path(workspace));
        Ok(store::remove_expired(workspace, retention_days))
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn locate(state: &mut State, roots: Roots<'_>, note: &Path) -> Option<Located> {
    let find = |roots: &[PathBuf]| {
        roots.iter().find_map(|root| {
            relative_inside(root, note).map(|relative| (root.clone(), relative))
        })
    };
    let (vault, relative) = find(&state.roots).or_else(|| {
        state.roots = roots();
        find(&state.roots)
    })?;
    Some(Located {
        key: identity(note),
        dir: store::history_dir(&vault, &relative),
        vault,
    })
}

fn relative_inside(root: &Path, path: &Path) -> Option<PathBuf> {
    strip_root(root, path)
        .filter(|relative| !relative.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Deny pre-existing directory aliases and malformed descendants for normal
/// history operations. The caller already holds PROCESS_HISTORY_FILES; a
/// cooperating Core cannot swap history directories during the operation.
/// External processes remain outside that guarantee.
fn allowed_history_directory(base: &Path, directory: &Path) -> bool {
    match store::checked_history_note_directory(base, directory) {
        Ok(()) => true,
        Err(error) => {
            report(directory, "заблокирована из-за небезопасного пути", &error.to_string());
            false
        }
    }
}

/// The intent record lives in the selected vault, rather than the private
/// app-data directory, so another Core can fail closed after a restart.
/// Scanning every known root also blocks cross-vault consumers that currently
/// have both workspaces registered. Separate-process admission remains a P0.
fn history_blocked(state: &HistoryFilesStateGuard<'_>, located: &Located) -> bool {
    if state.is_blocked(&located.key) {
        return true;
    }
    for root in &state.roots {
        match recovery::pending(root) {
            Ok(false) => {}
            Ok(true) => {
                report(root, "заблокирована незавершённым переносом", "persistent move intent");
                return true;
            }
            Err(error) => {
                report(root, "заблокирована повреждённым журналом переноса", &error.to_string());
                return true;
            }
        }
    }
    false
}

fn name_version(device: &str, dir: &Path, id: Option<String>, name: &str) -> Option<String> {
    let id = id.or_else(|| store::versions(dir).pop().map(|version| version.id))?;
    match store::write_name(dir, device, &id, name, SystemTime::now()) {
        Ok(()) => Some(id),
        Err(error) => {
            report(dir, "не записала название версии", &error.to_string());
            None
        }
    }
}

fn report(dir: &Path, what: &str, error: &str) {
    eprintln!("[aquilum:history] история {} {what}: {error}", dir.display());
}

fn device_id(app_data: &Path) -> String {
    let path = app_data.join(DEVICE_FILE);
    let stored = fs::read_to_string(&path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| value.len() == 8 && value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    if let Some(device) = stored {
        return device;
    }
    let device = uuid::Uuid::new_v4().simple().to_string()[..8].to_owned();
    if let Err(error) = fs::create_dir_all(app_data).and_then(|()| fs::write(&path, &device)) {
        eprintln!("[aquilum:history] идентификатор устройства не сохранён: {error}");
    }
    device
}

#[cfg(test)]
mod tests {
    use super::{HistoryService, NoteWrite};
    use crate::files::trash::{move_to_trash_impl, restore_from_trash_impl, trash_path};
    use crate::history::milestone::Source;
    use crate::history::store::{history_dir, versions};
    use std::fs;
    use std::path::{Path, PathBuf};

    struct Fixture {
        _directory: tempfile::TempDir,
        data: PathBuf,
        vault: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().unwrap();
            let root = crate::search::paths::canonical_path(directory.path());
            let vault = root.join("vault");
            fs::create_dir_all(&vault).unwrap();
            Self {
                data: root.join("data"),
                vault,
                _directory: directory,
            }
        }

        fn roots(&self) -> Vec<PathBuf> {
            vec![self.vault.clone()]
        }

        fn service(&self) -> HistoryService {
            HistoryService::new(&self.data)
        }

        fn save(&self, history: &HistoryService, note: &Path, text: &str, source: Source) {
            let before = history.disk_before(note, None);
            fs::create_dir_all(note.parent().unwrap()).unwrap();
            fs::write(note, text).unwrap();
            let hash = crate::files::document::hash_bytes(text.as_bytes());
            history.saved(&|| self.roots(), note, NoteWrite { disk_before: before, text, hash: &hash, source, name: None });
        }

        fn history_of(&self, relative: &str) -> PathBuf {
            history_dir(&self.vault, Path::new(relative))
        }
    }

    fn texts(dir: &Path) -> Vec<(String, Source)> {
        versions(dir)
            .into_iter()
            .map(|version| (fs::read_to_string(dir.join(&version.id)).unwrap(), version.source))
            .collect()
    }

    fn write<'a>(text: &'a str, source: Source, hash: &'a str, name: Option<&'a str>) -> NoteWrite<'a> {
        NoteWrite { disk_before: None, text, hash, source, name }
    }

    fn rename(history: &HistoryService, fixture: &Fixture, from: &Path, to: &Path) {
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::rename(from, to).unwrap();
        history.moved(&|| fixture.roots(), from, to);
    }

    #[test]
    fn history_service_serializes_saved_across_distinct_instances() {
        use super::PROCESS_HISTORY_FILES;
        use std::sync::{mpsc, Arc};
        use std::thread;
        use std::time::Duration;

        let fixture = Fixture::new();
        let note = fixture.vault.join("Shared.md");
        fs::write(&note, "existing file").unwrap();
        let vault = fixture.vault.clone();
        let app_data = fixture._directory.path().join("other-app-data");
        let first = Arc::new(fixture.service());
        let second = Arc::new(HistoryService::new(&app_data));
        let recorded = fixture.history_of("Shared.md");

        let held = PROCESS_HISTORY_FILES.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let saved = second.saved(
                &|| vec![vault.clone()],
                &note,
                NoteWrite {
                    disk_before: None,
                    text: "new version",
                    hash: "test-hash",
                    source: Source::Agent,
                    name: None,
                },
            );
            done_tx.send(saved).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "a second history service wrote without the process history lock");
        assert!(!recorded.exists(), "history bytes changed while the mutex was held");
        drop(held);

        assert!(done_rx.recv_timeout(Duration::from_secs(30)).unwrap());
        worker.join().unwrap();
        assert_eq!(versions(&recorded).len(), 1);
        drop(first);
    }

    #[test]
    fn history_service_serializes_legacy_migration_and_retention() {
        use super::PROCESS_HISTORY_FILES;
        use std::sync::{mpsc, Arc};
        use std::thread;
        use std::time::Duration;

        let fixture = Fixture::new();
        let old = fixture.vault.join(".quantum/history/Legacy.md/1_device_me.md");
        fs::create_dir_all(old.parent().unwrap()).unwrap();
        fs::write(&old, "legacy bytes").unwrap();
        let expected = fixture.history_of("Legacy.md").join("1_device_me.md");
        let vault = fixture.vault.clone();
        let first = Arc::new(fixture.service());
        let second = Arc::new(HistoryService::new(&fixture._directory.path().join("other-app-data")));

        let held = PROCESS_HISTORY_FILES.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_tx.send(second.maintain_retention(&vault, 0)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "migration ignored the shared history lock");
        assert!(old.exists());
        assert!(!expected.exists());
        drop(held);

        assert_eq!(done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap(), 0);
        worker.join().unwrap();
        assert_eq!(fs::read(&expected).unwrap(), b"legacy bytes");
        assert!(!old.exists());
        drop(first);
    }

    #[cfg(unix)]
    #[test]
    fn history_service_rejects_symlinked_history_root_for_reads_and_writes() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        let foreign = outside.path().join("history/Идея.md");
        fs::create_dir_all(&foreign).unwrap();
        let private = foreign.join("0000000000001_other_me.md");
        fs::write(&private, "private foreign history").unwrap();
        symlink(outside.path(), fixture.vault.join(".aquilum")).unwrap();
        let history = fixture.service();
        let note = fixture.vault.join("Идея.md");
        fs::write(&note, "local note").unwrap();
        assert!(!history.saved(&|| fixture.roots(), &note, write("local edit", Source::Agent, "hash", None)));
        assert_eq!(history.page(&|| fixture.roots(), &note, 0, 20).total, 0);
        assert!(history.name(&|| fixture.roots(), &note, None, "foreign").is_none());
        assert_eq!(fs::read_to_string(&private).unwrap(), "private foreign history");
        assert_eq!(fs::read_dir(&foreign).unwrap().count(), 1);
        fs::remove_file(fixture.vault.join(".aquilum")).unwrap();
        assert!(history.saved(&|| fixture.roots(), &note, write("local edit", Source::Agent, "hash", None)));
        assert_eq!(versions(&fixture.history_of("Идея.md")).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn history_service_rejects_nested_symlink_move_and_write_targets() {
        use std::os::unix::fs::symlink;
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        let history = fixture.service();
        let old = fixture.vault.join("Старое.md");
        fixture.save(&history, &old, "safe version", Source::Agent);
        let original_dir = fixture.history_of("Старое.md");
        let target = fixture.vault.join("Linked/Новая.md");
        let history_root = fixture.history_of("");
        symlink(outside.path(), history_root.join("Linked")).unwrap();

        history.moved(&|| fixture.roots(), &old, &target);
        assert_eq!(texts(&original_dir), [("safe version".to_owned(), Source::Agent)]);
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        assert!(!history.saved(&|| fixture.roots(), &target, write("do not leak", Source::Agent, "hash", None)));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn history_service_rejects_junction_root_and_nested_write_targets() {
        use std::process::Command;
        fn junction(link: &Path, target: &Path) {
            let output = Command::new("cmd")
                .arg("/C")
                .arg("mklink")
                .arg("/J")
                .arg(link)
                .arg(target)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "mklink /J failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let fixture = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        let history = fixture.service();
        let note = fixture.vault.join("Идея.md");
        fs::write(&note, "local").unwrap();
        let history_root = fixture.vault.join(".aquilum");
        junction(&history_root, outside.path());
        assert!(!history.saved(&|| fixture.roots(), &note, write("blocked", Source::Agent, "hash", None)));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_dir(&history_root).unwrap();

        let nested = fixture.history_of("");
        fs::create_dir_all(&nested).unwrap();
        junction(&nested.join("Идея.md"), outside.path());
        assert!(!history.saved(&|| fixture.roots(), &note, write("still blocked", Source::Agent, "hash", None)));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_dir(nested.join("Идея.md")).unwrap();
        assert!(history.saved(&|| fixture.roots(), &note, write("local", Source::Agent, "hash", None)));
        assert_eq!(versions(&fixture.history_of("Идея.md")).len(), 1);
    }

    #[test]
    fn history_failure_on_initial_baseline_preserves_original_for_retry() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Первый.md");
        fs::write(&note, "before primary save").unwrap();
        let captured = history.disk_before(&note, None);
        fs::write(&note, "first edit committed").unwrap();

        history.fail_once(HistoryFault::CreateVersion);
        let failed = history.try_saved(
            &|| fixture.roots(),
            &note,
            NoteWrite {
                disk_before: captured,
                text: "first edit committed",
                hash: "first-hash",
                source: Source::Me,
                name: None,
            },
        ).unwrap_err();
        assert_eq!(failed.action, "write baseline");
        assert!(versions(&fixture.history_of("Первый.md")).is_empty());
        let key = crate::search::paths::identity(&note);
        let state = history.lock();
        let pending = state.notes.get(&key).unwrap();
        assert!(pending.written_hash.is_none());
        assert!(pending.last_edit.is_none());
        assert_eq!(pending.pending_before.as_ref().unwrap().0, "before primary save");
        drop(state);

        let new_capture = history.disk_before(&note, None);
        fs::write(&note, "retry after failure").unwrap();
        assert!(history.try_saved(
            &|| fixture.roots(),
            &note,
            NoteWrite {
                disk_before: new_capture,
                text: "retry after failure",
                hash: "retry-hash",
                source: Source::Me,
                name: None,
            },
        ).unwrap());
        assert_eq!(
            texts(&fixture.history_of("Первый.md")),
            [
                ("before primary save".to_owned(), Source::Start),
                ("retry after failure".to_owned(), Source::Me),
            ]
        );
        assert!(history.lock().notes.get(&key).unwrap().pending_before.is_none());
    }

    #[test]
    fn history_failed_update_does_not_advance_hash_or_coalescing_state() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Черновик.md");
        assert!(history.try_saved(&|| fixture.roots(), &note, write("one", Source::Me, "hash-1", None)).unwrap());
        let key = crate::search::paths::identity(&note);
        let original_edit = history.lock().notes.get(&key).unwrap().last_edit;
        history.fail_once(HistoryFault::UpdateVersion);
        let error = history.try_saved(
            &|| fixture.roots(), &note, write("two", Source::Me, "hash-2", None)
        ).unwrap_err();
        assert_eq!(error.action, "update version");
        assert_eq!(texts(&fixture.history_of("Черновик.md")), [("one".to_owned(), Source::Me)]);
        {
            let state = history.lock();
            let note = state.notes.get(&key).unwrap();
            assert_eq!(note.written_hash.as_deref(), Some("hash-1"));
            assert_eq!(note.last_edit, original_edit);
        }
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write("three", Source::Me, "hash-3", None)
        ).unwrap());
        assert_eq!(texts(&fixture.history_of("Черновик.md")), [("three".to_owned(), Source::Me)]);
    }

    #[test]
    fn history_failed_name_write_is_not_reported_as_changed_and_can_retry() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Имена.md");
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write("original", Source::Agent, "hash-1", None)
        ).unwrap());
        history.fail_once(HistoryFault::NameVersion);
        let error = history.try_saved(
            &|| fixture.roots(), &note, write("original", Source::Agent, "hash-1", Some("draft"))
        ).unwrap_err();
        assert_eq!(error.action, "write version name");
        assert!(history.page(&|| fixture.roots(), &note, 0, 10).versions[0].name.is_none());
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write("original", Source::Agent, "hash-1", Some("draft"))
        ).unwrap());
        assert_eq!(
            history.page(&|| fixture.roots(), &note, 0, 10).versions[0].name.as_deref(),
            Some("draft")
        );
    }

    #[test]
    fn history_failed_revert_never_claims_removed_version() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Откат.md");
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write("first", Source::Agent, "hash-1", None)
        ).unwrap());
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write("second", Source::Me, "hash-2", None)
        ).unwrap());
        history.fail_once(HistoryFault::RemoveVersion);
        let error = history.try_saved(
            &|| fixture.roots(), &note, write("first", Source::Me, "hash-3", None)
        ).unwrap_err();
        assert_eq!(error.action, "remove version");
        assert_eq!(texts(&fixture.history_of("Откат.md")).len(), 2);
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write("first", Source::Me, "hash-3", None)
        ).unwrap());
        assert_eq!(texts(&fixture.history_of("Откат.md")), [("first".to_owned(), Source::Agent)]);
    }

    #[test]
    fn typing_creates_baseline_and_coalesces_subsequent_typing() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Идея.md");
        fs::write(&note, "было").unwrap();

        fixture.save(&history, &note, "было и", Source::Me);
        fixture.save(&history, &note, "было и стало", Source::Me);

        assert_eq!(
            texts(&fixture.history_of("Идея.md")),
            [("было".to_owned(), Source::Start), ("было и стало".to_owned(), Source::Me)]
        );
    }

    #[test]
    fn every_agent_edit_becomes_a_version() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Агент.md");

        fixture.save(&history, &note, "от агента", Source::Agent);
        fixture.save(&history, &note, "от агента и снова", Source::Agent);

        assert_eq!(
            texts(&fixture.history_of("Агент.md")),
            [("от агента".to_owned(), Source::Agent), ("от агента и снова".to_owned(), Source::Agent)]
        );
    }

    #[test]
    fn history_cross_vault_service_direct_call_cannot_write_source_only_intent() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let source = fixture.vault.join("Direct.md");
        fixture.save(&history, &source, "keep this version", Source::Agent);
        let target_vault = fixture._directory.path().join("second-vault");
        fs::create_dir_all(&target_vault).unwrap();
        let target_vault = fs::canonicalize(target_vault).unwrap();
        let target = target_vault.join("Direct.md");
        let before = fixture.history_of("Direct.md");
        assert_eq!(versions(&before).len(), 1);

        let err = history.try_moved(
            &|| vec![fixture.vault.clone(), target_vault.clone()],
            &source,
            &target,
        ).unwrap_err();
        assert_eq!(err.action, "move history");
        assert!(!err.may_have_partial_files);
        assert!(source.exists(), "the primary note was not touched");
        assert_eq!(versions(&before).len(), 1);
        assert!(!history_dir(&target_vault, Path::new("Direct.md")).exists());
        assert!(!crate::history::recovery::marker_path(&fixture.vault).exists());
        assert!(!crate::history::recovery::marker_path(&target_vault).exists());
    }

    #[test]
    fn history_recovery_manifest_survives_state_loss_and_verifies_all_version_bytes() {
        use super::{HistoryFault, PROCESS_HISTORY_FILES};
        let fixture = Fixture::new();
        let first = fixture.service();
        let old = fixture.vault.join("Durable.md");
        let new = fixture.vault.join("Durable-new.md");
        fixture.save(&first, &old, "first version", Source::Agent);
        fixture.save(&first, &old, "second version", Source::Agent);
        fs::rename(&old, &new).unwrap();

        first.fail_once(HistoryFault::MoveAfterOneFile);
        assert!(first.try_moved(&|| fixture.roots(), &old, &new).is_err());
        assert!(crate::history::recovery::pending(&fixture.vault).unwrap());

        // Simulate process restart by discarding only THIS transfer's
        // process-local quarantine, not other concurrent tests' entries.
        let mut process = PROCESS_HISTORY_FILES.lock().unwrap();
        process.blocked.remove(&crate::search::paths::identity(&old));
        process.blocked.remove(&crate::search::paths::identity(&new));
        drop(process);
        let reopened = fixture.service();
        assert_eq!(reopened.page(&|| fixture.roots(), &new, 0, 10).total, 0);
        assert_eq!(
            reopened.try_saved(&|| fixture.roots(), &new, write("forbidden", Source::Agent, "hash", None))
                .unwrap_err().action,
            "history relocation pending"
        );
        assert!(reopened.maintain_retention(&fixture.vault, 30).is_err());

        let target = fixture.history_of("Durable-new.md");
        let moved = fs::read_dir(&target).unwrap().next().unwrap().unwrap().path();
        let saved = fs::read(&moved).unwrap();
        fs::remove_file(&moved).unwrap();

        // Source may be absent or incomplete: it is NEVER evidence of
        // completion unless all recorded filenames and BLAKE3 digests match.
        assert!(reopened.try_moved(&|| fixture.roots(), &old, &new).is_err());
        assert!(crate::history::recovery::pending(&fixture.vault).unwrap());
        fs::write(&moved, &saved).unwrap();

        reopened.try_moved(&|| fixture.roots(), &old, &new).unwrap();
        assert!(!crate::history::recovery::pending(&fixture.vault).unwrap());
        assert_eq!(reopened.page(&|| fixture.roots(), &new, 0, 10).total, 2);
        assert!(!fixture.history_of("Durable.md").exists());
        assert!(reopened.maintain_retention(&fixture.vault, 0).is_ok());
    }

    #[test]
    fn history_corrupt_recovery_marker_denies_reopened_core() {
        use super::{HistoryFault, PROCESS_HISTORY_FILES};
        let fixture = Fixture::new();
        let first = fixture.service();
        let old = fixture.vault.join("Broken.md");
        let new = fixture.vault.join("Broken-new.md");
        fixture.save(&first, &old, "version", Source::Agent);
        fs::rename(&old, &new).unwrap();
        first.fail_once(HistoryFault::MoveBefore);
        assert!(first.try_moved(&|| fixture.roots(), &old, &new).is_err());
        let marker = crate::history::recovery::marker_path(&fixture.vault);
        fs::write(&marker, b"{not-json").unwrap();

        let mut process = PROCESS_HISTORY_FILES.lock().unwrap();
        process.blocked.remove(&crate::search::paths::identity(&old));
        process.blocked.remove(&crate::search::paths::identity(&new));
        drop(process);
        let second = fixture.service();
        assert!(crate::history::recovery::pending(&fixture.vault).is_err());
        assert!(second.page(&|| fixture.roots(), &new, 0, 10).versions.is_empty());
        assert!(second.try_saved(&|| fixture.roots(), &new, write("blocked", Source::Agent, "hash", None)).is_err());
        assert!(second.try_moved(&|| fixture.roots(), &old, &new).is_err());
        assert!(marker.exists(), "corrupt recovery marker must never be deleted automatically");
    }

    #[test]
    fn history_move_failure_does_not_retarget_cached_version_path() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let old = fixture.vault.join("Old.md");
        let new = fixture.vault.join("New.md");
        fixture.save(&history, &old, "first", Source::Agent);
        fixture.save(&history, &old, "second", Source::Agent);
        fs::rename(&old, &new).unwrap();
        let original = fixture.history_of("Old.md");
        let target = fixture.history_of("New.md");
        let source_key = crate::search::paths::identity(&old);
        let target_key = crate::search::paths::identity(&new);

        history.fail_once(HistoryFault::MoveBefore);
        let error = history.try_moved(&|| fixture.roots(), &old, &new).unwrap_err();
        assert_eq!(error.action, "move history");
        assert!(error.may_have_partial_files);
        assert!(original.is_dir());
        assert!(!target.exists());
        assert_eq!(history.lock().notes.get(&source_key).unwrap().dir, original);
        assert!(!history.lock().notes.contains_key(&target_key));
        assert!(history.page(&|| fixture.roots(), &new, 0, 10).versions.is_empty());
        assert_eq!(history.try_saved(
            &|| fixture.roots(), &new, write("should not fork", Source::Agent, "hash", None)
        ).unwrap_err().action, "history relocation pending");

        history.try_moved(&|| fixture.roots(), &old, &new).unwrap();
        assert!(!original.exists());
        assert_eq!(texts(&target), [
            ("first".to_owned(), Source::Agent), ("second".to_owned(), Source::Agent)
        ]);
        assert_eq!(history.lock().notes.get(&target_key).unwrap().dir, target);
        assert!(!history.lock().notes.contains_key(&source_key));
        assert_eq!(history.page(&|| fixture.roots(), &new, 0, 10).total, 2);
    }

    #[test]
    fn history_move_partial_failure_quarantines_both_trees_across_cores() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let first = fixture.service();
        let second = HistoryService::new(&fixture._directory.path().join("other-data"));
        let old = fixture.vault.join("Partial.md");
        let new = fixture.vault.join("Recovered.md");
        fixture.save(&first, &old, "version one", Source::Agent);
        fixture.save(&first, &old, "version two", Source::Agent);
        fs::rename(&old, &new).unwrap();
        let source = fixture.history_of("Partial.md");
        let target = fixture.history_of("Recovered.md");

        first.fail_once(HistoryFault::MoveAfterOneFile);
        let failure = first.try_moved(&|| fixture.roots(), &old, &new).unwrap_err();
        assert!(failure.may_have_partial_files);
        assert_eq!(versions(&source).len(), 1);
        assert_eq!(versions(&target).len(), 1);
        assert_eq!(first.page(&|| fixture.roots(), &old, 0, 10).total, 0);
        assert_eq!(second.page(&|| fixture.roots(), &new, 0, 10).total, 0);
        assert_eq!(second.try_saved(
            &|| fixture.roots(), &new, write("cannot fork", Source::Agent, "new-hash", None)
        ).unwrap_err().action, "history relocation pending");
        assert!(first.try_moved(
            &|| fixture.roots(), &old, &fixture.vault.join("Unrelated.md")
        ).is_err());
        assert!(first.maintain_retention(&fixture.vault, 30).is_err());
        let unrelated = fixture._directory.path().join("other-workspace");
        fs::create_dir_all(&unrelated).unwrap();
        assert_eq!(second.maintain_retention(&unrelated, 0).unwrap(), 0);

        first.try_moved(&|| fixture.roots(), &old, &new).unwrap();
        assert!(!source.exists());
        assert_eq!(texts(&target), [
            ("version one".to_owned(), Source::Agent),
            ("version two".to_owned(), Source::Agent),
        ]);
        assert_eq!(second.page(&|| fixture.roots(), &new, 0, 10).total, 2);
        assert_eq!(first.maintain_retention(&fixture.vault, 0).unwrap(), 0);
    }

    #[test]
    fn history_move_trash_failure_keeps_original_cache_until_retry() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("TrashMe.md");
        fixture.save(&history, &note, "kept history", Source::Agent);
        let trashed = move_to_trash_impl(&fixture.vault, &note).unwrap().root;
        let original = fixture.history_of("TrashMe.md");
        let target = history_dir(&trash_path(&fixture.vault), Path::new("TrashMe.md"));
        let source_key = crate::search::paths::identity(&note);

        history.fail_once(HistoryFault::MoveBefore);
        let err = history.try_trashed(&|| fixture.roots(), &note, &trashed).unwrap_err();
        assert_eq!(err.action, "trash history");
        assert!(original.exists());
        assert!(!target.exists());
        assert_eq!(history.lock().notes.get(&source_key).unwrap().dir, original);
        history.try_trashed(&|| fixture.roots(), &note, &trashed).unwrap();
        assert!(!original.exists());
        assert_eq!(texts(&target), [("kept history".to_owned(), Source::Agent)]);
        assert!(history.lock().notes.get(&source_key).is_none());
        assert_eq!(history.lock().notes.get(&crate::search::paths::identity(&trashed)).unwrap().dir, target);
    }

    #[test]
    fn history_move_restore_failure_keeps_trash_cache_until_retry() {
        use super::HistoryFault;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("RestoreMe.md");
        fixture.save(&history, &note, "recover this", Source::Agent);
        let trashed = move_to_trash_impl(&fixture.vault, &note).unwrap().root;
        history.try_trashed(&|| fixture.roots(), &note, &trashed).unwrap();
        let restored = restore_from_trash_impl(&fixture.vault, &trashed).unwrap().root;
        let trash_dir = history_dir(&trash_path(&fixture.vault), Path::new("RestoreMe.md"));
        let restored_dir = fixture.history_of("RestoreMe.md");
        let source_key = crate::search::paths::identity(&trashed);

        history.fail_once(HistoryFault::MoveBefore);
        let err = history.try_restored(&|| fixture.roots(), &trashed, &restored).unwrap_err();
        assert_eq!(err.action, "restore history");
        assert!(trash_dir.exists());
        assert!(!restored_dir.exists());
        assert_eq!(history.lock().notes.get(&source_key).unwrap().dir, trash_dir);
        assert_eq!(history.try_saved(
            &|| fixture.roots(), &restored, write("must wait", Source::Agent, "hash", None)
        ).unwrap_err().action, "history relocation pending");

        history.try_restored(&|| fixture.roots(), &trashed, &restored).unwrap();
        assert!(!trash_dir.exists());
        assert_eq!(texts(&restored_dir), [("recover this".to_owned(), Source::Agent)]);
        assert!(history.lock().notes.get(&source_key).is_none());
        assert_eq!(history.page(&|| fixture.roots(), &restored, 0, 10).total, 1);
    }

    #[test]
    fn the_history_follows_a_renamed_note() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let old = fixture.vault.join("Старое.md");
        let new = fixture.vault.join("Проекты").join("Новое.md");
        fixture.save(&history, &old, "текст", Source::Agent);

        rename(&history, &fixture, &old, &new);
        fixture.save(&history, &new, "текст дальше", Source::Agent);

        assert_eq!(
            texts(&fixture.history_of("Проекты/Новое.md")),
            [("текст".to_owned(), Source::Agent), ("текст дальше".to_owned(), Source::Agent)]
        );
        assert!(!fixture.history_of("Старое.md").exists());
    }

    #[test]
    fn a_rename_that_changes_only_the_case_keeps_the_history() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let old = fixture.vault.join("идея.md");
        let new = fixture.vault.join("Идея.md");
        fixture.save(&history, &old, "текст", Source::Agent);
        fixture.save(&history, &old, "текст!", Source::Agent);

        rename(&history, &fixture, &old, &new);

        assert_eq!(
            texts(&fixture.history_of("Идея.md")),
            [("текст".to_owned(), Source::Agent), ("текст!".to_owned(), Source::Agent)]
        );
        let names = fs::read_dir(fixture.history_of(""))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(names, ["Идея.md"]);
    }

    #[test]
    fn a_renamed_folder_leaves_no_empty_history_folders() {
        let fixture = Fixture::new();
        let history = fixture.service();
        for name in ["А.md", "Б.md"] {
            let note = fixture.vault.join("Старая").join(name);
            fixture.save(&history, &note, name, Source::Agent);
            fixture.save(&history, &note, &format!("{name}!"), Source::Agent);
        }
        fs::rename(fixture.vault.join("Старая"), fixture.vault.join("Новая")).unwrap();

        for name in ["А.md", "Б.md"] {
            history.moved(&|| fixture.roots(), &fixture.vault.join("Старая").join(name), &fixture.vault.join("Новая").join(name));
        }

        assert!(!fixture.history_of("Старая").exists());
        assert_eq!(
            texts(&fixture.history_of("Новая/Б.md")),
            [("Б.md".to_owned(), Source::Agent), ("Б.md!".to_owned(), Source::Agent)]
        );
    }

    #[test]
    fn the_history_goes_to_the_trash_with_its_note_and_comes_back() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Удаляемая.md");
        fixture.save(&history, &note, "начало", Source::Agent);
        fixture.save(&history, &note, "начало и середина", Source::Me);

        let trashed = move_to_trash_impl(&fixture.vault, &note).unwrap().root;
        history.trashed(&|| fixture.roots(), &note, &trashed);

        let in_trash = history_dir(&trash_path(&fixture.vault), Path::new("Удаляемая.md"));
        assert_eq!(
            texts(&in_trash),
            [("начало".to_owned(), Source::Agent), ("начало и середина".to_owned(), Source::Me)]
        );
        assert!(!fixture.vault.join(".aquilum").exists());

        let restored = restore_from_trash_impl(&fixture.vault, &trashed).unwrap().root;
        history.restored(&|| fixture.roots(), &trashed, &restored);
        fixture.save(&history, &restored, "начало и середина от агента", Source::Agent);

        assert_eq!(
            texts(&fixture.history_of("Удаляемая.md")),
            [
                ("начало".to_owned(), Source::Agent),
                ("начало и середина".to_owned(), Source::Me),
                ("начало и середина от агента".to_owned(), Source::Agent),
            ]
        );
        assert!(!trash_path(&fixture.vault).join(".aquilum").exists());
    }

    #[test]
    fn a_second_copy_in_the_trash_keeps_its_own_history() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Идея.md");
        let mut trashed = Vec::new();
        for text in ["первая", "вторая"] {
            fixture.save(&history, &note, text, Source::Agent);
            fixture.save(&history, &note, &format!("{text}!"), Source::Agent);
            let moved = move_to_trash_impl(&fixture.vault, &note).unwrap().root;
            history.trashed(&|| fixture.roots(), &note, &moved);
            trashed.push(moved);
        }

        let restored = restore_from_trash_impl(&fixture.vault, &trashed[1]).unwrap().root;
        history.restored(&|| fixture.roots(), &trashed[1], &restored);

        assert_eq!(
            texts(&fixture.history_of("Идея.md")),
            [("вторая".to_owned(), Source::Agent), ("вторая!".to_owned(), Source::Agent)]
        );
        let first = history_dir(&trash_path(&fixture.vault), Path::new("Идея.md"));
        assert_eq!(
            texts(&first),
            [("первая".to_owned(), Source::Agent), ("первая!".to_owned(), Source::Agent)]
        );
    }

    #[test]
    fn a_restart_keeps_what_was_on_disk_as_the_starting_text() {
        let fixture = Fixture::new();
        let note = fixture.vault.join("Заметка.md");
        let first = fixture.service();
        fixture.save(&first, &note, "первая", Source::Agent);
        fixture.save(&first, &note, "до закрытия", Source::Agent);

        fs::write(&note, "правка снаружи").unwrap();
        let second = fixture.service();
        fixture.save(&second, &note, "правка снаружи и моя", Source::Me);

        assert_eq!(
            texts(&fixture.history_of("Заметка.md")),
            [
                ("первая".to_owned(), Source::Agent),
                ("до закрытия".to_owned(), Source::Agent),
                ("правка снаружи и моя".to_owned(), Source::Me)
            ]
        );
    }

    #[test]
    fn the_page_lists_past_versions_newest_first_and_a_version_comes_with_the_one_before() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Заметка.md");
        for text in ["первая", "вторая", "третья"] {
            fixture.save(&history, &note, text, Source::Agent);
        }

        let page = history.page(&|| fixture.roots(), &note, 0, 1);

        assert_eq!(page.total, 3);
        let third = history.version(&|| fixture.roots(), &note, Some(&page.versions[0].id)).unwrap();
        assert_eq!((third.text.as_str(), third.previous.as_deref()), ("третья", Some("вторая")));
        let second = history.page(&|| fixture.roots(), &note, 1, 1).versions.remove(0);
        assert_eq!(
            history.version(&|| fixture.roots(), &note, Some(&second.id)).unwrap().previous.as_deref(),
            Some("первая")
        );
        let first = history.page(&|| fixture.roots(), &note, 2, 1).versions.remove(0);
        assert_eq!(history.version(&|| fixture.roots(), &note, Some(&first.id)).unwrap().previous, None);
    }

    #[test]
    fn reading_progress_after_a_restart_adds_no_version() {
        let fixture = Fixture::new();
        let note = fixture.vault.join("Книга.md");
        let first = fixture.service();
        fixture.save(&first, &note, "книга", Source::Agent);
        fixture.save(&first, &note, "книга и заметка", Source::Agent);
        fs::write(&note, "книга и заметка позиция 1").unwrap();

        let history = fixture.service();
        fixture.save(&history, &note, "книга и заметка позиция 2", Source::Reading);

        assert_eq!(
            texts(&fixture.history_of("Книга.md")),
            [("книга".to_owned(), Source::Agent), ("книга и заметка".to_owned(), Source::Agent)]
        );
    }

    #[test]
    fn a_restore_creates_a_restored_version_immediately() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Заметка.md");
        let roots = || fixture.roots();
        fixture.save(&history, &note, "первая", Source::Agent);
        fixture.save(&history, &note, "первая и правка", Source::Me);
        let first = history.page(&roots, &note, 1, 1).versions.remove(0);

        fixture.save(&history, &note, "первая", Source::Restore(first.at_ms));

        assert_eq!(
            texts(&fixture.history_of("Заметка.md")),
            [
                ("первая".to_owned(), Source::Agent),
                ("первая и правка".to_owned(), Source::Me),
                ("первая".to_owned(), Source::Restore(first.at_ms)),
            ]
        );
        let page = history.page(&roots, &note, 0, 10);
        assert_eq!(page.versions[0].from_ms, Some(first.at_ms));
        let current = page.versions.iter().map(|version| version.is_current).collect::<Vec<_>>();
        assert_eq!(current, [true, false, true]);
    }

    #[test]
    fn a_version_gets_a_name_and_latest_can_be_named() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Заметка.md");
        let roots = || fixture.roots();
        fixture.save(&history, &note, "первая", Source::Agent);
        fixture.save(&history, &note, "вторая", Source::Agent);
        let first = history.page(&roots, &note, 1, 1).versions.remove(0);

        assert_eq!(history.name(&roots, &note, Some(&first.id), "Начало"), Some(first.id.clone()));
        assert_eq!(history.name(&roots, &note, Some("0000000000000_aaaaaaaa_me.md"), "Нет"), None);
        let live = history.name(&roots, &note, None, "Черновик").unwrap();

        let page = history.page(&roots, &note, 0, 10);
        let named = page.versions.iter().map(|version| version.name.clone()).collect::<Vec<_>>();
        assert_eq!(named, [Some("Черновик".to_owned()), Some("Начало".to_owned())]);
        assert_eq!(page.versions[0].id, live);
    }

    #[test]
    fn a_write_with_a_name_names_the_version_it_produces() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Заметка.md");
        let roots = || fixture.roots();

        history.saved(&roots, &note, write("от агента", Source::Agent, "1", Some("План")));
        history.saved(&roots, &note, write("от агента", Source::Agent, "1", Some("План, ещё раз")));

        let page = history.page(&roots, &note, 0, 10);
        assert_eq!(page.total, 1);
        assert_eq!(page.versions[0].name.as_deref(), Some("План, ещё раз"));
    }

    #[test]
    fn only_a_listed_version_can_be_read() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Заметка.md");
        fixture.save(&history, &note, "текст", Source::Agent);
        fs::write(fixture.vault.join("Секрет.md"), "не отсюда").unwrap();

        assert!(history.version(&|| fixture.roots(), &note, Some("../../../Секрет.md")).is_none());
        assert!(history.version(&|| fixture.roots(), &note, Some("0000000000000_aaaaaaaa_me.md")).is_none());
    }

    #[test]
    fn p0g3d05_cold_history_version_is_bound_to_one_note_after_service_reopen() {
        let fixture = Fixture::new();
        let note = fixture.vault.join("BoundCold.md");
        let text = "exact immutable external snapshot";
        let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
        let previous = "preserved local version";
        fs::write(&note,text).unwrap();
        let history = fixture.service();
        let result = history.try_saved_impl(
            &||fixture.roots(),&note,
            NoteWrite {
                disk_before:Some((
                    previous.to_owned(),std::time::SystemTime::now(),
                )),
                text,hash:&hash,source:Source::External,name:None,
            },
            true,true,
        ).unwrap();
        let strict = result.verified_version.unwrap();
        let id = strict.version_id().to_owned();
        drop(history);
        // New HistoryService state: no in-memory version witness survives.
        let fresh = fixture.service();
        let cold = fresh.inspect_cold_external_version(
            &||fixture.roots(),&note,&id,&hash,text.len() as u64,
        ).unwrap();
        assert_eq!(cold.registered_root,fixture.vault);
        assert_eq!(cold.note,note);
        assert_eq!(cold.version_id,id);
        assert_eq!(cold.content_hash,hash);
        assert_eq!(cold.content_length,text.len() as u64);
        let unrelated = fixture.vault.join("Other.md");
        fs::write(&unrelated,text).unwrap();
        assert!(fresh.inspect_cold_external_version(
            &||fixture.roots(),&unrelated,&id,&hash,text.len() as u64,
        ).is_err(),"a different note cannot inherit this version");
        assert!(fresh.inspect_cold_external_version(
            &||vec![fixture.vault.join("not-registered")],
            &note,&id,&hash,text.len() as u64,
        ).is_err(),"a different registration cannot inherit this version");
        assert_eq!(fs::read_to_string(&note).unwrap(),text);
    }

    #[test]
    fn p0g3d05_cold_history_rejects_mutation_length_hash_and_non_external_source() {
        let fixture = Fixture::new();
        let note = fixture.vault.join("ColdTamper.md");
        let text = "external version to be inspected";
        let history = fixture.service();
        fixture.save(&history,&note,text,Source::External);
        let id=history.page(&||fixture.roots(),&note,0,1)
            .versions[0].id.clone();
        let hash=blake3::hash(text.as_bytes()).to_hex().to_string();
        let read=|h:&HistoryService| h.inspect_cold_external_version(
            &||fixture.roots(),&note,&id,&hash,text.len() as u64,
        );
        assert!(read(&history).is_ok());
        assert!(history.inspect_cold_external_version(
            &||fixture.roots(),&note,&id,&hash,32*1024*1024+1,
        ).is_err());
        assert!(history.inspect_cold_external_version(
            &||fixture.roots(),&note,&id,&"e".repeat(64),text.len() as u64,
        ).is_err());
        let file=fixture.history_of("ColdTamper.md").join(&id);
        fs::write(&file,"X".repeat(text.len())).unwrap();
        let cold=fixture.service();
        assert!(read(&cold).is_err(),
            "equal-length history mutation must fail after cold-open");

        let local=fixture.vault.join("LocallyTyped.md");
        fixture.save(&history,&local,"local",Source::Agent);
        let version=history.page(&||fixture.roots(),&local,0,1)
            .versions[0].id.clone();
        let localhash=blake3::hash(b"local").to_hex().to_string();
        assert!(cold.inspect_cold_external_version(
            &||fixture.roots(),&local,&version,&localhash,5,
        ).is_err(),"an Agent/Me version must never masquerade as external");
    }

    #[cfg(unix)]
    #[test]
    fn p0g3d05_cold_history_refuses_hardlink_and_symlink_version_files() {
        use std::os::unix::fs::symlink;
        let fixture=Fixture::new();
        let history=fixture.service();
        let note=fixture.vault.join("Aliased.md");
        let text="external history snapshot";
        fixture.save(&history,&note,text,Source::External);
        let id=history.page(&||fixture.roots(),&note,0,1)
            .versions[0].id.clone();
        let dir=fixture.history_of("Aliased.md");
        let version=dir.join(&id);
        let hash=blake3::hash(text.as_bytes()).to_hex().to_string();
        let extra=dir.join("hardlinked-other-name");
        fs::hard_link(&version,&extra).unwrap();
        assert!(history.inspect_cold_external_version(
            &||fixture.roots(),&note,&id,&hash,text.len() as u64,
        ).is_err());
        fs::remove_file(&extra).unwrap();
        fs::remove_file(&version).unwrap();
        let external=fixture.vault.join("External.md");
        fs::write(&external,text).unwrap();
        symlink(&external,&version).unwrap();
        assert!(history.inspect_cold_external_version(
            &||fixture.roots(),&note,&id,&hash,text.len() as u64,
        ).is_err());
    }

    #[test]
    fn p0g3b_strict_history_proof_identifies_new_version_under_history_lock() {
        use super::HistorySaveEffect;
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("BoundHistory.md");
        let original = "protected old revision";
        let updated = "fresh external S3 candidate for strict history proof";
        fs::write(&note, updated).unwrap();
        let new_hash = blake3::hash(updated.as_bytes()).to_hex().to_string();
        let outcome = history.try_saved_impl(
            &|| fixture.roots(), &note,
            NoteWrite {
                disk_before: Some((
                    original.to_owned(), std::time::SystemTime::now(),
                )),
                text: updated, hash: &new_hash,
                source: Source::External, name: Some("candidate"),
            }, true, true,
        ).expect("one current snapshot must be proven");
        let id = match &outcome.effect {
            HistorySaveEffect::Created(id) => id.clone(),
            unexpected => panic!("expected new snapshot: {unexpected:?}"),
        };
        let proof = outcome.verified_version.as_ref().unwrap();
        assert_eq!(proof.version_id, id);
        assert_eq!(proof.content_hash, new_hash);
        assert_eq!(proof.content_length, updated.len());
        assert_eq!(fs::read(fixture.history_of("BoundHistory.md").join(&id))
            .unwrap(), updated.as_bytes());
        assert!(outcome.changed());
    }

    #[test]
    fn p0g3b_strict_history_rejects_local_source_before_creating_version() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("LocallyMutable.md");
        let text = "local typed revision would be coalescible";
        fs::write(&note, text).unwrap();
        let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
        // This would normally be CreateNew; the strict, non-authorizing S3
        // proof may only observe an external save. The regular editor still
        // retains its existing local coalescing path.
        let result = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(text, Source::Me, &hash, None), true, true,
        );
        assert!(result.is_err(), "Source::Me would permit later coalescing");
        assert!(versions(&fixture.history_of("LocallyMutable.md")).is_empty());
        assert!(history.try_saved(
            &|| fixture.roots(), &note,
            write(text, Source::Me, &hash, None),
        ).unwrap(), "ordinary local save remains supported");
    }

    #[test]
    fn p0g3b_history_observation_is_not_permanent_archive() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("ExternalObservation.md");
        let text = "external revision verified once";
        fs::write(&note, text).unwrap();
        let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
        let result = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(text, Source::External, &hash, None), true, true,
        ).expect("strictly create history version");
        let proof = result.verified_version.expect("observed exact version");
        let dir = fixture.history_of("ExternalObservation.md");
        assert_eq!(proof.content_hash, hash);
        // A same-UID writer can mutate even a once-verified version after the
        // history lock is released. A future native receipt MUST retain its
        // own protected bytes and independently recheck before remote ACK.
        fs::write(dir.join(&proof.version_id), b"changed after receipt").unwrap();
        assert!(super::HistoryService::verify_history_version(
            &dir, &proof.version_id, text, &hash,
        ).is_err(), "later history mutation invalidates the old observation");
    }

    #[test]
    fn p0g3b_strict_history_proof_refuses_skip_and_label_only() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("SameContent.md");
        let text = "already saved";
        fs::write(&note, text).unwrap();
        let hash = blake3::hash(text.as_bytes()).to_hex().to_string();
        assert!(history.try_saved(
            &|| fixture.roots(), &note, write(text, Source::External, &hash, None),
        ).unwrap());
        // Normal legacy saves preserve their existing quiet/skip result.
        assert!(!history.try_saved(
            &|| fixture.roots(), &note, write(text, Source::External, &hash, None),
        ).unwrap());
        let strict = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(text, Source::External, &hash, None), true, true,
        );
        assert!(strict.is_err(), "Skip has no unique snapshot commit receipt");
        let label_only = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(text, Source::External, &hash, Some("label only")), true, true,
        );
        assert!(label_only.is_err(), "A label is not a content-version receipt");
        assert_eq!(versions(&fixture.history_of("SameContent.md")).len(), 1);
        assert!(crate::history::store::names(
            &fixture.history_of("SameContent.md")
        ).is_empty(), "strict refusal must happen before writing a label");
    }

    #[test]
    fn p0g3b_strict_history_proof_refuses_coalesced_update() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Coalesced.md");
        let original = "first local edit";
        let revised = "first local edit, amended immediately";
        fs::write(&note, original).unwrap();
        let first_hash = blake3::hash(original.as_bytes()).to_hex().to_string();
        assert!(history.try_saved(
            &|| fixture.roots(), &note,
            write(original, Source::Me, &first_hash, None),
        ).unwrap());
        fs::write(&note, revised).unwrap();
        let second_hash = blake3::hash(revised.as_bytes()).to_hex().to_string();
        let result = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(revised, Source::Me, &second_hash, None), true, true,
        );
        assert!(result.is_err(),
            "a mutable/coalesced version cannot prove immutable history identity");
        let stored = versions(&fixture.history_of("Coalesced.md"));
        assert_eq!(stored.len(), 1,
            "normal history coalescing is preserved even when strict evidence is refused");
        assert_eq!(fs::read_to_string(
            fixture.history_of("Coalesced.md").join(&stored[0].id),
        ).unwrap(), original,
            "strict refusal must precede any mutation of the old snapshot");
        // The normal editor remains allowed to coalesce later, without using
        // the strict receipt-only path or breaking its old milestone rules.
        assert!(history.try_saved(
            &|| fixture.roots(), &note,
            write(revised, Source::Me, &second_hash, None),
        ).unwrap());
        assert_eq!(versions(&fixture.history_of("Coalesced.md")).len(), 1);
        assert_eq!(fs::read_to_string(
            fixture.history_of("Coalesced.md").join(&stored[0].id),
        ).unwrap(), revised);
    }

    #[test]
    fn p0g3b_strict_history_proof_never_deletes_reverted_version() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("Revert.md");
        let earlier = "first history text";
        let later = "second history text";
        fs::write(&note, earlier).unwrap();
        let first_hash = blake3::hash(earlier.as_bytes()).to_hex().to_string();
        assert!(history.try_saved(
            &|| fixture.roots(), &note,
            write(earlier, Source::External, &first_hash, None),
        ).unwrap());
        fs::write(&note, later).unwrap();
        let second_hash = blake3::hash(later.as_bytes()).to_hex().to_string();
        assert!(history.try_saved(
            &|| fixture.roots(), &note,
            write(later, Source::Me, &second_hash, None),
        ).unwrap());
        let dir = fixture.history_of("Revert.md");
        let saved = versions(&dir);
        assert_eq!(saved.len(), 2);
        let result = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(earlier, Source::Me, &first_hash, None), true, true,
        );
        assert!(result.is_err(), "reversion is not an immutable new snapshot");
        let after = versions(&dir);
        assert_eq!(after.len(), 2, "strict rejection must not delete a version");
        assert_eq!(after[1].id, saved[1].id);
        assert_eq!(fs::read_to_string(dir.join(&saved[1].id)).unwrap(), later);
    }

    #[test]
    fn p0g3b_strict_history_proof_refuses_forged_expected_hash_after_write() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let note = fixture.vault.join("TamperedDigest.md");
        let text = "trusted bytes in local history";
        fs::write(&note, text).unwrap();
        let forged_hash = "a".repeat(64);
        let result = history.try_saved_impl(
            &|| fixture.roots(), &note,
            write(text, Source::External, &forged_hash, None), true, true,
        );
        assert!(result.is_err(), "a wrong digest must never attest a snapshot");
        assert!(versions(&fixture.history_of("TamperedDigest.md")).len() > 0,
            "bytes may already be durable: refusal must be inspection-only");
        // The method does not return a verified version on uncertain write.
    }

    #[test]
    fn a_note_outside_known_workspaces_has_no_history() {
        let fixture = Fixture::new();
        let history = fixture.service();
        let stray = fixture.data.join("Чужая.md");
        fs::create_dir_all(&fixture.data).unwrap();
        fs::write(&stray, "текст").unwrap();

        history.saved(&|| fixture.roots(), &stray, write("текст!", Source::Agent, "hash", None));

        assert!(!fixture.vault.join(".aquilum").exists());
        assert!(!fixture.data.join(".aquilum").exists());
    }

    #[test]
    fn the_device_id_is_kept_between_launches() {
        let fixture = Fixture::new();
        let first = fixture.service();
        let second = fixture.service();
        assert_eq!(first.device, second.device);
        assert_eq!(first.device.len(), 8);
    }

    #[cfg(feature = "sync-s3")]
    #[test]
    fn enrollment_waits_for_history_files_and_late_mutations_observe_hold() {
        use super::PROCESS_HISTORY_FILES;
        use crate::files::error::FileCommandError;
        use std::sync::{mpsc, Arc};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;
        use std::time::Duration;

        let fixture = Fixture::new();
        let history = Arc::new(fixture.service());
        let policy = Arc::new(AtomicBool::new(false));
        history.bind_guarded_participant_policy(Arc::clone(&policy));

        // A history operation that was already admitted owns the shared
        // mutex. The enrolling Core MUST wait for that operation to finish
        // before switching the live participant flag and exposing new routes.
        let held_history = PROCESS_HISTORY_FILES.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_service = Arc::clone(&history);
        let worker_policy = Arc::clone(&policy);
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            let result = worker_service.with_participant_enrollment_fence(|| {
                worker_policy.store(true, Ordering::Release);
                Ok::<(), FileCommandError>(())
            });
            done_tx.send(result).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "participant switched while a history writer still held the mutex");
        assert!(!policy.load(Ordering::Acquire));
        drop(held_history);
        done_rx.recv_timeout(Duration::from_secs(20)).unwrap().unwrap();
        worker.join().unwrap();
        assert!(policy.load(Ordering::Acquire));

        let note = fixture.vault.join("NoNewHistory.md");
        fs::write(&note, "unmodified").unwrap();
        let blocked = history.try_saved(
            &|| fixture.roots(), &note,
            NoteWrite {
                disk_before: None,
                text: "rejected",
                hash: "never-publish",
                source: Source::Agent,
                name: None,
            },
        );
        assert!(blocked.is_err());
        assert!(!fixture.history_of("NoNewHistory.md").exists());
        assert_eq!(fs::read_to_string(&note).unwrap(), "unmodified");
    }
}
