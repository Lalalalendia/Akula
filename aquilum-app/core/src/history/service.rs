use super::milestone::{self, Before, Incoming, MilestonePlan, Recorded, Source};
use super::store;
use crate::files::document::read_text;
use crate::files::error::FileCommandError;
use crate::files::relocate::{move_folder, remove_empty_ancestors, Placement};
use crate::files::trash::trash_path;
use crate::search::paths::{identity, strip_root};
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard};
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
            #[cfg(test)]
            next_fault: Mutex::new(None),
        }
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
        let now = SystemTime::now();
        let Some(mut state) = self.files_lock() else {
            return Err(HistorySaveFailure::new("history lock", "poisoned"));
        };
        let Some(located) = locate(&mut state, roots, note) else {
            return Ok(false);
        };
        if state.is_blocked(&located.key) {
            return Err(HistorySaveFailure::new("history relocation pending", "history transfer must be retried before recording a new version"));
        }
        if !allowed_history_directory(&located.vault, &located.dir) {
            return Err(HistorySaveFailure::new("history path", "unsafe history directory"));
        }
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
        let quiet = matches!(&plan, MilestonePlan::Quiet);
        let dir = &located.dir;
        let outcome = (|| -> Result<bool, HistorySaveFailure> {
            match plan {
                MilestonePlan::Quiet => Ok(false),
                MilestonePlan::Skip => match (write.name, versions.last()) {
                    (Some(name), Some(latest)) => {
                        self.write_name(dir, &latest.id, name, now)
                            .map_err(|error| HistorySaveFailure::new("write version name", error))?;
                        Ok(true)
                    }
                    _ => Ok(false),
                },
                MilestonePlan::RevertToPrevious { remove_id } => {
                    let removed = self.remove_version_and_names(dir, &remove_id);
                    // A later label deletion may fail after unlinking the
                    // version. Keep coalescing state aligned with the disk.
                    if !dir.join(&remove_id).exists() {
                        entry.last_edit = None;
                    }
                    removed.map_err(|error| HistorySaveFailure::new("remove version", error))?;
                    Ok(true)
                }
                MilestonePlan::UpdateLatest { id } => {
                    self.update_version(dir, &id, write.text)
                        .map_err(|error| HistorySaveFailure::new("update version", error))?;
                    entry.last_edit = Some(now);
                    if let Some(name) = write.name {
                        self.write_name(dir, &id, name, now)
                            .map_err(|error| HistorySaveFailure::new("write version name", error))?;
                    }
                    Ok(true)
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
                    Ok(true)
                }
            }
        })();
        match outcome {
            Ok(changed) => {
                // A quiet reader update must not discard a failed baseline
                // awaiting the next real save.
                if !quiet || entry.pending_before.is_none() {
                    entry.written_hash = Some(write.hash.to_owned());
                    entry.pending_before = None;
                }
                Ok(changed)
            }
            Err(error) => {
                if versions.is_empty() && entry.pending_before.is_none() {
                    entry.pending_before = write.disk_before;
                }
                Err(error)
            }
        }
    }

    fn write_version(&self, dir: &Path, snapshot: &milestone::Snapshot) -> Result<PathBuf, FileCommandError> {
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
        let Some(mut state) = self.files_lock() else {
            return None;
        };
        let located = locate(&mut state, roots, note)?;
        if state.is_blocked(&located.key) || !allowed_history_directory(&located.vault, &located.dir) {
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
        if state.is_blocked(&located.key) || !allowed_history_directory(&located.vault, &located.dir) {
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
        if state.is_blocked(&located.key) || !allowed_history_directory(&located.vault, &located.dir) {
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
        let Some(mut state) = self.files_lock() else {
            return Err(HistoryMoveFailure::new(action, "history mutex poisoned", false));
        };
        let (Some(source), Some(target)) = (
            locate(&mut state, roots, from),
            locate(&mut state, roots, to),
        ) else {
            return Ok(());
        };
        state.assert_retry_pair(&source.key, &target.key, action)?;
        if !allowed_history_directory(&source.vault, &source.dir)
            || !allowed_history_directory(&target.vault, &target.dir)
        {
            state.quarantine(&source.key, &target.key, &[source.vault.as_path(), target.vault.as_path()], "unsafe history directory");
            return Err(HistoryMoveFailure::new(action, "unsafe history directory", false));
        }
        if let Err(error) = self.move_history(&source.dir, &target.dir, Placement::Workspace, &source.vault) {
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
        let Some(mut state) = self.files_lock() else {
            return Err(HistoryMoveFailure::new(action, "history mutex poisoned", false));
        };
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
        if let Err(error) = self.move_history(&located.dir, &target, Placement::Trash, &located.vault) {
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
        let Some(mut state) = self.files_lock() else {
            return Err(HistoryMoveFailure::new(action, "history mutex poisoned", false));
        };
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
        if let Err(error) = self.move_history(&source, &located.dir, Placement::Workspace, &trash) {
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
        let guard = PROCESS_HISTORY_FILES
            .lock()
            .map_err(|_| "process history-files lock poisoned; refusing cleanup")?;
        // A failed transfer in one vault must not stall retention in every
        // unrelated vault. The global lock is still held only during work.
        if guard.blocked.values().any(|block| block.affected_roots.iter().any(|root| {
            root.as_path() == workspace || trash_path(root).as_path() == workspace
        })) {
            return Err("history relocation pending in selected workspace; retention deferred");
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
}
