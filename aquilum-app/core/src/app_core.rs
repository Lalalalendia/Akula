use crate::documents::{DocumentEvent, DocumentHub, SaveFailed};
use crate::files::gate::{NoteHistoryChanged, NoteHistoryMoveFailed, NoteHistorySaveFailed, Relocation};
use crate::files::error::FileCommandError;
use crate::files::watcher::{WatchBatch, WatchScope, WatchSink, WorkspaceWatcher};
use crate::history::HistoryService;
use crate::mcp::active::ActiveNote;
use crate::mcp::bridge::Navigation;
use crate::mcp::{McpServer, McpStatus};
use crate::search::paths::is_markdown;
use crate::search::{ChangeNotifier, IndexNotifier, IndexRevision, SearchService};
use crate::settings::SettingsManager;
use crate::ui_state::UiStateService;
use crate::wikixiv::WikixivService;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
#[cfg(feature = "sync-s3")]
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

// Conservatively serialize all Core-managed mutations and protected original
// captures within this process, even when two distinct Core instances use
// the same workspace. External processes still require an OS-level guard.
static PROCESS_FILE_MUTATIONS: Mutex<()> = Mutex::new(());
/// An ordinary note's primary bytes and HistoryService may be separate
/// mutex stages. Hold a read lease across BOTH, while activation obtains
/// an exclusive write fence before publishing participant HOLD.
#[cfg(feature = "sync-s3")]
static PROCESS_NOTE_PUBLICATIONS: RwLock<()> = RwLock::new(());

/// What the core tells its host.
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum CoreEvent {
    DocumentChanged(DocumentEvent),
    DocumentSaved(DocumentEvent),
    DocumentMissing(DocumentEvent),
    DocumentSaveFailed(SaveFailed),
    NotesRelocated(Relocation),
    NoteHistoryChanged(NoteHistoryChanged),
    NoteHistorySaveFailed(NoteHistorySaveFailed),
    NoteHistoryMoveFailed(NoteHistoryMoveFailed),
    WorkspaceChanged(Vec<String>),
    ContentChanged(Vec<String>),
    WorkspaceRescanned,
    LinksChanged(IndexRevision),
    Navigate(Navigation),
}

impl CoreEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::DocumentChanged(_) => "document-changed",
            Self::DocumentSaved(_) => "document-saved",
            Self::DocumentMissing(_) => "document-missing",
            Self::DocumentSaveFailed(_) => "document-save-failed",
            Self::NotesRelocated(_) => "notes-relocated",
            Self::NoteHistoryChanged(_) => "note-history-changed",
            Self::NoteHistorySaveFailed(_) => "note-history-save-failed",
            Self::NoteHistoryMoveFailed(_) => "note-history-move-failed",
            Self::WorkspaceChanged(_) => "workspace-changed",
            Self::ContentChanged(_) => "content-changed",
            Self::WorkspaceRescanned => "workspace-rescanned",
            Self::LinksChanged(_) => "links-changed",
            Self::Navigate(_) => "mcp-navigate",
        }
    }
}

/// Receives core events. Implementations must not block and must not call back into the core
/// synchronously: events are sent from the core's own threads, sometimes while it holds locks.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: CoreEvent);
}

impl<F: Fn(CoreEvent) + Send + Sync> EventSink for F {
    fn emit(&self, event: CoreEvent) {
        self(event)
    }
}

/// A background task of the host died before finishing (panic or shutdown).
#[derive(Debug)]
pub struct TaskFailed(pub String);

impl std::fmt::Display for TaskFailed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Every service of the app, wired together, with no knowledge of the UI that hosts it.
pub struct Core {
    #[cfg(feature = "sync-s3")]
    installed_host_custody_db: Option<PathBuf>,
    pub settings: SettingsManager,
    pub ui_state: UiStateService,
    pub search: SearchService,
    pub history: HistoryService,
    pub documents: DocumentHub,
    /// Process-wide serialization shared by all Core instances, ordinary
    /// File Gate writes and read-only protected-original capture. It is NOT a
    /// cross-process OS lock or trusted write authorization.
    pub(crate) file_mutations: &'static Mutex<()>,
    /// Explicitly armed, per-Core cooperative editor save routes. No UI
    /// activation, implicit S3 bootstrap or remote sync write permission.
    #[cfg(feature = "sync-s3")]
    pub(crate) guarded_editors: Mutex<Vec<crate::sync::guarded_editor::ActiveEditorGuard>>,
    /// Never reuse a live one-shot prewrite ticket from an earlier Core in
    /// the same thread, even if the host reconstructs a Core without exiting.
    #[cfg(feature = "sync-s3")]
    pub(crate) guarded_instance_nonce: uuid::Uuid,
    pub wikixiv: WikixivService,
    pub watcher: WorkspaceWatcher,
    /// The note open in the UI, reported by the host; MCP tools use it for «эта заметка».
    pub active_note: ActiveNote,
    pub mcp: McpServer,
    events: Arc<dyn EventSink>,
}

/// A non-cloneable, lexical capability valid only while the installer-wide
/// SQLite enrollment epoch is held by Core::with_unenrolled_host_epoch.
/// It is NOT remote S3 write authority, a protected-custody token or evidence
/// against uncooperative same-user writers.
#[cfg(feature = "sync-s3")]
pub(crate) struct HostUnenrolledEpochPermit<'a> {
    core: &'a Core,
}

#[cfg(feature = "sync-s3")]
impl HostUnenrolledEpochPermit<'_> {
    pub(crate) fn binds_history(&self, history: &HistoryService) -> bool {
        std::ptr::eq(&self.core.history, history)
    }
}

impl Core {
    /// Opens all stores under `app_data_dir`, which the host picks.
    pub fn open(app_data_dir: &Path, events: Arc<dyn EventSink>) -> Arc<Self> {
        Self::open_inner(app_data_dir, events, #[cfg(feature = "sync-s3")] None)
    }

    /// P0-B2a *explicit* host-bound Core for a cooperating installer-owned
    /// source. Unlike Core::open, ordinary legacy mutations will fail closed
    /// unless individually routed through the shared enrollment epoch.
    /// The caller MUST obtain this path from the trusted host, not appdata.
    /// Full production native startup still needs to make it mandatory.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn open_with_installed_host(
        app_data_dir: &Path, host_custody_db: &Path, events: Arc<dyn EventSink>,
    ) -> Arc<Self> {
        Self::open_inner(app_data_dir, events, Some(host_custody_db.to_path_buf()))
    }

    fn open_inner(
        app_data_dir: &Path,
        events: Arc<dyn EventSink>,
        #[cfg(feature = "sync-s3")] installed_host_custody_db: Option<PathBuf>,
    ) -> Arc<Self> {
        let core = Arc::new_cyclic(|this: &Weak<Core>| {
            let changed = Arc::clone(&events);
            let notifier: ChangeNotifier = Arc::new(move |paths| {
                changed.emit(CoreEvent::WorkspaceChanged(path_names(&paths)));
            });
            let indexed = Arc::clone(&events);
            let index_notifier: IndexNotifier = Arc::new(move |revision| {
                indexed.emit(CoreEvent::LinksChanged(revision));
            });
            let watched = Weak::clone(this);
            let watch_sink: WatchSink = Arc::new(move |batch| {
                if let Some(core) = watched.upgrade() {
                    core.ingest_watch(batch);
                }
            });
            Core {
                #[cfg(feature = "sync-s3")]
                installed_host_custody_db,
                settings: SettingsManager::new(app_data_dir),
                ui_state: UiStateService::open(&app_data_dir.join("ui-state.sqlite3")),
                search: SearchService::new(app_data_dir.join("search-v2"), notifier, index_notifier),
                history: HistoryService::new(app_data_dir),
                documents: DocumentHub::new(&app_data_dir.join("documents.sqlite3")),
                file_mutations: &PROCESS_FILE_MUTATIONS,
                #[cfg(feature = "sync-s3")]
                guarded_editors: Mutex::new(Vec::new()),
                #[cfg(feature = "sync-s3")]
                guarded_instance_nonce: uuid::Uuid::new_v4(),
                wikixiv: WikixivService::new(),
                watcher: WorkspaceWatcher::new(watch_sink),
                active_note: ActiveNote::default(),
                mcp: McpServer::new(),
                events,
            }
        });
        // Bind HistoryService to the SAME live/cold-start participant HOLD
        // before publishing the Core to hosts, watchers or editor workers.
        core.history.bind_guarded_participant_policy(core.documents.guarded_history_policy());
        #[cfg(feature = "sync-s3")]
        if core.installed_host_custody_db.is_some() {
            // Every direct HistoryService mutator must refuse until routed
            // through the host epoch; do so before exposing workers/hosts.
            core.history.require_installed_host_epoch();
        }
        core.documents.start(Arc::downgrade(&core));
        core
    }

    /// Acquire only around actual File Gate disk operations. Never emit
    /// callbacks, reconcile the editor or write history while holding this.
    pub(crate) fn file_mutation_guard(&self) -> Result<MutexGuard<'_, ()>, FileCommandError> {
        self.file_mutations.lock().map_err(|_| FileCommandError::Task {
            message: "Core File Gate interlock poisoned; refusing mutation".into(),
        })
    }

    /// Keep a legacy note publication (primary bytes + history snapshot)
    /// within a single cooperative admission epoch. Callers MUST drop this
    /// guard before UI callbacks, document reconciliation or notifications.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn ordinary_note_publication_lease(
        &self,
    ) -> Result<RwLockReadGuard<'static, ()>, FileCommandError> {
        PROCESS_NOTE_PUBLICATIONS.read().map_err(|_| FileCommandError::Task {
            message: "note publication epoch poisoned; refusing ordinary save".into(),
        })
    }

    /// Exclusive barrier for an explicit protected-root activation, acquired
    /// AFTER DocumentHub prewrite arming and BEFORE File Gate/HistoryFiles.
    /// Waits for any already-admitted ordinary note primary+history attempt
    /// to finish. This is process-local, not an interprocess custody lock.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn ordinary_note_enrollment_fence(
        &self,
    ) -> Result<RwLockWriteGuard<'static, ()>, FileCommandError> {
        PROCESS_NOTE_PUBLICATIONS.write().map_err(|_| FileCommandError::Task {
            message: "note publication epoch poisoned; refusing enrollment".into(),
        })
    }

    /// Mutating File Gate entrypoints must refuse a pending or invalid history
    /// transfer BEFORE changing the primary bytes. This deliberately checks
    /// every registered workspace, mirroring HistoryService's conservative
    /// process-wide recovery hold. The read-only File Gate admission path still
    /// uses file_mutation_guard() so forensic inspection remains possible.
    ///
    /// This is a pathname/SQLite HOLD check under an in-process mutex, NOT
    /// an OS interprocess lock or permission for a sync overwrite/delete.
    pub(crate) fn file_mutation_guard_for_write(&self) -> Result<MutexGuard<'_, ()>, FileCommandError> {
        self.file_mutation_guard_for_write_mode(false, false)
    }

    /// INTERNAL ONLY: the caller must already hold a verified, pinned
    /// external OS custody lock across the entire primary write + history.
    /// No other File Gate operation may use this exception. This is not a
    /// remote sync overwrite/delete capability or a generic write permit.
    pub(crate) fn file_mutation_guard_for_custody_write(&self) -> Result<MutexGuard<'_, ()>, FileCommandError> {
        self.file_mutation_guard_for_write_mode(true, false)
    }

    #[cfg(feature = "sync-s3")]
    pub(crate) fn requires_installed_host_epoch(&self) -> bool {
        self.installed_host_custody_db.is_some()
    }

    /// Only the still-held host SQLite enrollment epoch can invoke this
    /// exception, never a normal File Gate entrypoint or a network reader.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn file_mutation_guard_for_host_write(
        &self, permit: &HostUnenrolledEpochPermit<'_>,
    ) -> Result<MutexGuard<'_, ()>, FileCommandError> {
        if !std::ptr::eq(self, permit.core)
            || self.documents.requires_guarded_participant_hold()
        {
            return Err(FileCommandError::Task {
                message: "host ordinary mutation is HOLD: wrong Core or locally enrolled participant".into(),
            });
        }
        self.file_mutation_guard_for_write_mode(false, true)
    }

    fn file_mutation_guard_for_write_mode(
        &self, proven_custody: bool, host_epoch_held: bool,
    ) -> Result<MutexGuard<'_, ()>, FileCommandError> {
        let guard = self.file_mutation_guard()?;
        #[cfg(feature = "sync-s3")]
        if !proven_custody && !host_epoch_held && self.installed_host_custody_db.is_some() {
            return Err(FileCommandError::Task {
                message: "host custody is HOLD: mutation has no held shared enrollment epoch".into(),
            });
        }
        #[cfg(not(feature = "sync-s3"))]
        let _ = host_epoch_held;
        if !proven_custody && self.documents.requires_guarded_participant_hold() {
            return Err(FileCommandError::Task {
                message: "File Gate is HOLD: an S3 workspace was enrolled or its appdata registry is unavailable; ordinary mutations require explicit custody admission".into(),
            });
        }
        let roots = self.ui_state.workspace_roots().map_err(|error| FileCommandError::Task {
            message: format!(
                "Cannot read registered workspaces before File Gate mutation: {error:?}"
            ),
        })?;
        for root in roots {
            match crate::history::pending_history_relocation(&root) {
                Ok(false) => {}
                Ok(true) => {
                    return Err(FileCommandError::Task {
                        message: format!(
                            "File Gate mutation refused: unfinished history transfer HOLD in {}",
                            root.display()
                        ),
                    });
                }
                Err(error) => {
                    return Err(FileCommandError::Task {
                        message: format!(
                            "File Gate mutation refused: unreadable or invalid history recovery state in {}: {error}",
                            root.display()
                        ),
                    });
                }
            }
        }
        Ok(guard)
    }

    /// P0-B2a: a host-BOUND Core's limited ordinary write/create adapter.
    /// The SQLite BEGIN IMMEDIATE shares enrollment's writer lock, and is
    /// retained across the entire primary + history closure. All other
    /// File Gate and direct HistoryService mutations remain HOLD in this mode.
    ///
    /// This path is supplied by the installer contract, never from appdata;
    /// Core::open (legacy) still bypasses this policy and MUST be migrated
    /// before #251's real alternate-appdata release blocker can close.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn with_unenrolled_host_epoch<T>(
        &self,
        target: &Path,
        operation: impl FnOnce(&HostUnenrolledEpochPermit<'_>) -> Result<T, FileCommandError>,
    ) -> Result<T, FileCommandError> {
        use crate::sync::filegate_lock_probe::PinnedProbeRoot;
        use crate::sync::host_custody_registry::{
            PhysicalObjectId, SharedHostCustodyRegistry,
        };
        use std::fs;

        let refused = |message: String| FileCommandError::Task { message };
        let db = self.installed_host_custody_db.as_ref()
            .ok_or_else(|| refused("no installer-issued shared custody source".into()))?;
        let roots = self.ui_state.workspace_roots()
            .map_err(|error| refused(format!("host root registration unavailable: {error:?}")))?;
        // Only a registered, unique, existing physical workspace can be
        // observed. Reject symlinks (including nested directory aliases) and
        // paths that resolve outside the selected root.
        let target_meta = fs::symlink_metadata(target);
        let resolved = match target_meta {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(refused("host-bound writer rejects symlink leaf".into()));
            }
            Ok(_) => fs::canonicalize(target)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = target.parent()
                    .ok_or_else(|| refused("target lacks a parent directory".into()))?;
                fs::canonicalize(parent)?.join(target.file_name()
                    .ok_or_else(|| refused("target lacks a file name".into()))?)
            }
            Err(error) => return Err(error.into()),
        };
        let mut owners = Vec::new();
        for root in roots {
            if let Ok(relative) = target.strip_prefix(&root) {
                let canonical = fs::canonicalize(&root)?;
                if canonical.join(relative) != resolved {
                    return Err(refused("host-bound path changed or used an ambiguous alias".into()));
                }
                owners.push(root);
            }
        }
        if owners.len() != 1 {
            return Err(refused("host-bound writer needs exactly one registered workspace".into()));
        }
        let root = owners.pop().unwrap();
        let pinned = PinnedProbeRoot::open(&root)?;
        let (device, file) = pinned.identity();
        let mut registry = SharedHostCustodyRegistry::open_existing(db)
            .map_err(|error| refused(format!("shared custody registry unavailable: {error:?}")))?;
        let mut entered = false;
        let outcome = registry.with_unenrolled_epoch(
            &root, PhysicalObjectId { device, file }, || {
                entered = true;
                pinned.recheck_root()?;
                if self.documents.requires_guarded_participant_hold() {
                    return Err(refused("local participant already requires custody HOLD".into()));
                }
                let permit = HostUnenrolledEpochPermit { core: self };
                let result = operation(&permit);
                // If replacement is detected after publication, never tell
                // the caller to blindly retry or assume unchanged bytes.
                if pinned.recheck_root().is_err() {
                    return Err(FileCommandError::NeedsReview {
                        message: "physical root replaced during shared host publication; inspect before retry".into(),
                    });
                }
                result
            },
        );
        match outcome {
            Ok(result) => result,
            Err(error) if entered => Err(FileCommandError::NeedsReview {
                message: format!("host epoch commit uncertain after operation; recover before retry: {error:?}"),
            }),
            Err(error) => Err(refused(format!("host custody admission refused: {error:?}"))),
        }
    }

    /// MCP permanent trash purge must not race an ordinary File Gate restore
    /// or protected-original capture. This locks only the filesystem purge;
    /// no history/editor callbacks are made while the mutex is held.
    ///
    /// The purge can delay other in-process writes for large trash folders.
    /// It is not a cross-process lock or a trusted sync-delete executor.
    pub fn empty_trash_guarded(&self, workspace: &Path) -> Result<u64, FileCommandError> {
        let _guard = self.file_mutation_guard_for_write()?;
        crate::files::trash::empty_trash_impl(workspace)
    }

    pub fn emit(&self, event: CoreEvent) {
        self.events.emit(event);
    }

    /// Starts, restarts or stops the MCP server to match the saved settings.
    pub fn apply_mcp_settings(self: &Arc<Self>) -> McpStatus {
        self.mcp.apply(self, &self.settings.get_config().mcp)
    }

    /// Stops background work before the host exits: unsaved edits are written, MCP and the
    /// watcher stop.
    pub fn shutdown(&self) {
        self.documents.flush_all(self);
        self.mcp.shutdown();
        self.watcher.stop();
    }

    pub fn known_roots(&self) -> Vec<PathBuf> {
        self.ui_state.workspace_roots().unwrap_or_default()
    }

    fn ingest_watch(&self, batch: WatchBatch) {
        match batch.scope {
            WatchScope::Content => self.emit(CoreEvent::ContentChanged(path_names(&batch.paths))),
            WatchScope::Rescan if batch.paths.is_empty() => self.emit(CoreEvent::WorkspaceRescanned),
            WatchScope::Listing | WatchScope::Structure | WatchScope::Rescan => self.emit(CoreEvent::WorkspaceChanged(path_names(&batch.paths))),
        }
        let rescan = batch.scope == WatchScope::Rescan;
        let notes: Vec<PathBuf> = if rescan { batch.paths } else { batch.paths.into_iter().filter(|path| is_markdown(path)).collect() };
        if notes.is_empty() && !rescan {
            return;
        }
        self.documents.reconcile_paths(self, &notes);
        self.search.ingest_watch(notes, rescan);
    }
}

fn path_names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|path| path.to_string_lossy().into_owned()).collect()
}

#[cfg(test)]
#[path = "app_core_tests.rs"]
mod tests;
