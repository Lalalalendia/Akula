use crate::documents::{DocumentEvent, DocumentHub, SaveFailed};
use crate::files::gate::{FileGateInterprocessLock, NoteHistoryChanged, NoteHistoryMoveFailed, NoteHistorySaveFailed, Relocation};
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
#[cfg(test)]
use std::sync::TryLockError;
#[cfg(feature = "host-custody")]
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

// Conservatively serialize Core-managed mutations and protected original
// captures within this process and across Akula processes of the same OS user.
static PROCESS_FILE_MUTATIONS: FileMutationInterlock = FileMutationInterlock::new();
/// An ordinary note's primary bytes and HistoryService may be separate
/// mutex stages. Hold a read lease across BOTH, while activation obtains
/// an exclusive write fence before publishing participant HOLD.
#[cfg(feature = "host-custody")]
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

/// Local mutex plus a stable OS file lock shared by independent Akula
/// processes. The OS lock coordinates writers; it does not authenticate or
/// authorize them.
pub(crate) struct FileMutationInterlock {
    process: Mutex<()>,
}

impl FileMutationInterlock {
    const fn new() -> Self {
        Self {
            process: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> Result<FileMutationGuard<'_>, FileMutationLockError> {
        let process = self
            .process
            .lock()
            .map_err(|_| FileMutationLockError::Poisoned)?;
        let interprocess =
            FileGateInterprocessLock::acquire().map_err(FileMutationLockError::Interprocess)?;
        Ok(FileMutationGuard {
            process: Some(process),
            _interprocess: interprocess,
        })
    }

    #[cfg(test)]
    pub(crate) fn try_lock(
        &self,
    ) -> Result<FileMutationGuard<'_>, TryLockError<MutexGuard<'_, ()>>> {
        let process = self.process.try_lock()?;
        let interprocess = match FileGateInterprocessLock::try_acquire() {
            Ok(Some(interprocess)) => interprocess,
            Ok(None) => {
                drop(process);
                return Err(TryLockError::WouldBlock);
            }
            Err(error) => panic!("interprocess File Gate lock probe failed: {error}"),
        };
        Ok(FileMutationGuard {
            process: Some(process),
            _interprocess: interprocess,
        })
    }

    #[cfg(test)]
    pub(crate) fn try_process_only_lock(
        &self,
    ) -> Result<MutexGuard<'_, ()>, TryLockError<MutexGuard<'_, ()>>> {
        self.process.try_lock()
    }
}

#[derive(Debug)]
pub(crate) enum FileMutationLockError {
    Poisoned,
    Interprocess(std::io::Error),
}

impl std::fmt::Display for FileMutationLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Poisoned => formatter.write_str("process-local File Gate mutex is poisoned"),
            Self::Interprocess(error) => {
                write!(formatter, "interprocess File Gate lock failed: {error}")
            }
        }
    }
}

pub(crate) struct FileMutationGuard<'a> {
    process: Option<MutexGuard<'a, ()>>,
    _interprocess: FileGateInterprocessLock,
}

impl FileMutationGuard<'_> {
    /// Keep the cross-process File Gate epoch through history publication
    /// while releasing the shorter process-local mutex before callbacks.
    pub(crate) fn release_process_lock(&mut self) {
        self.process.take();
    }
}

/// Every service of the app, wired together, with no knowledge of the UI that hosts it.
pub struct Core {
    #[cfg(feature = "host-custody")]
    installed_host_custody_db: Option<PathBuf>,
    /// Mandatory compiled policy survives missing or damaged shared ledger.
    /// Never infer local fallback from an empty Option.
    #[cfg(feature = "host-custody")]
    installed_host_required: bool,
    pub settings: SettingsManager,
    pub ui_state: UiStateService,
    pub search: SearchService,
    pub history: HistoryService,
    pub documents: DocumentHub,
    /// Shared File Gate interlock for all Core instances. It combines a
    /// process-local mutex with a cooperative same-user OS lock; it is not a
    /// trusted write authorization.
    pub(crate) file_mutations: &'static FileMutationInterlock,
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
#[cfg(feature = "host-custody")]
pub(crate) struct HostUnenrolledEpochPermit<'a> {
    core: &'a Core,
}

#[cfg(feature = "host-custody")]
impl HostUnenrolledEpochPermit<'_> {
    pub(crate) fn binds_history(&self, history: &HistoryService) -> bool {
        std::ptr::eq(&self.core.history, history)
    }
}

impl Core {
    /// Open a Core. The separately selected installed-host product mode
    /// always requires the ONE build-time custody ledger for ALL processes.
    /// A missing path is a persistent read-only HOLD, not legacy File Gate.
    /// Runtime environment, appdata, and workspace settings cannot override it.
    pub fn open(app_data_dir: &Path, events: Arc<dyn EventSink>) -> Arc<Self> {
        #[cfg(feature = "sync-installed-host")]
        {
            let trusted = option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.file_name().is_some());
            return Self::open_inner(app_data_dir, events, trusted, true);
        }
        #[cfg(not(feature = "sync-installed-host"))]
        Self::open_inner(app_data_dir, events, None, false)
    }

    /// P0-B2a *explicit* host-bound Core for a cooperating installer-owned
    /// source. Unlike Core::open, ordinary legacy mutations will fail closed
    /// unless individually routed through the shared enrollment epoch.
    /// The caller MUST obtain this path from the trusted host, not appdata.
    /// Full production native startup still needs to make it mandatory.
    #[cfg(feature = "host-custody")]
    pub fn open_with_installed_host(
        app_data_dir: &Path, host_custody_db: &Path, events: Arc<dyn EventSink>,
    ) -> Arc<Self> {
        // Compiled-installed binaries must not let even this constructor
        // substitute a caller-provisioned empty private SQLite registry.
        #[cfg(feature = "sync-installed-host")]
        let trusted = option_env!("AKULA_INSTALLED_HOST_CUSTODY_DB")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && path.file_name().is_some())
            .filter(|path| path == host_custody_db);
        #[cfg(not(feature = "sync-installed-host"))]
        let trusted = host_custody_db.is_absolute().then(|| host_custody_db.to_path_buf());
        Self::open_inner(app_data_dir, events, trusted, true)
    }

    fn open_inner(
        app_data_dir: &Path,
        events: Arc<dyn EventSink>,
        installed_host_custody_db: Option<PathBuf>,
        installed_host_required: bool,
    ) -> Arc<Self> {
        #[cfg(not(feature = "host-custody"))]
        let _ = (&installed_host_custody_db, installed_host_required);
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
                #[cfg(feature = "host-custody")]
                installed_host_custody_db,
                #[cfg(feature = "host-custody")]
                installed_host_required,
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
        #[cfg(feature = "host-custody")]
        if core.installed_host_required {
            // Every direct HistoryService mutator must refuse until routed
            // through the host epoch; do so before exposing workers/hosts.
            core.history.require_installed_host_epoch();
        }
        core.documents.start(Arc::downgrade(&core));
        core
    }

    /// Acquire the shared File Gate interlock. The process-local mutex is for
    /// short disk sections; the OS lock may remain held through history.
    pub(crate) fn file_mutation_guard(&self) -> Result<FileMutationGuard<'_>, FileCommandError> {
        self.file_mutations
            .lock()
            .map_err(|error| FileCommandError::Task {
                message: format!(
                    "Core File Gate interlock unavailable; refusing mutation: {error}"
                ),
            })
    }

    /// Keep a legacy note publication (primary bytes + history snapshot)
    /// within a single cooperative admission epoch. Callers MUST drop this
    /// guard before UI callbacks, document reconciliation or notifications.
    #[cfg(feature = "host-custody")]
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
    #[cfg(feature = "host-custody")]
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
    /// This is a pathname/SQLite HOLD check under the shared File Gate lock,
    /// NOT permission for a sync overwrite/delete.
    pub(crate) fn file_mutation_guard_for_write(&self) -> Result<FileMutationGuard<'_>, FileCommandError> {
        self.file_mutation_guard_for_write_mode(false, false)
    }

    /// INTERNAL ONLY: the caller must already hold a verified, pinned
    /// external OS custody lock across the entire primary write + history.
    /// No other File Gate operation may use this exception. This is not a
    /// remote sync overwrite/delete capability or a generic write permit.
    pub(crate) fn file_mutation_guard_for_custody_write(&self) -> Result<FileMutationGuard<'_>, FileCommandError> {
        // The S3 custody permit is scoped to appdata/OS lock, not to the
        // installer-wide SQLite epoch. Forbid this independent bypass in an
        // installed-host binary until the verified host writer exists.
        #[cfg(feature = "host-custody")]
        if self.installed_host_required {
            return Err(FileCommandError::Task {
                message: "host policy HOLD: S3 custody writer lacks shared host admission".into(),
            });
        }
        self.file_mutation_guard_for_write_mode(true, false)
    }

    #[cfg(feature = "host-custody")]
    pub(crate) fn requires_installed_host_epoch(&self) -> bool {
        self.installed_host_required
    }
    /// The only installed-host registry opener used by BOTH ordinary File Gate
    /// publications and enrolled S3 activation. A witnessing build must read
    /// the independently provisioned expected IDs *before* opening SQLite.
    /// Missing or changed witness always means HOLD, never an empty roster.
    #[cfg(feature = "host-custody")]
    fn open_installed_host_registry(
        &self,
    ) -> Result<crate::sync::host_custody_registry::SharedHostCustodyRegistry, FileCommandError> {
        use crate::sync::host_custody_registry::SharedHostCustodyRegistry;

        let refused = |message| FileCommandError::Task { message };
        let db = self.installed_host_custody_db.as_ref()
            .ok_or_else(|| refused("host custody HOLD: installer ledger missing".into()))?;

        #[cfg(feature = "sync-installed-host-witness")]
        {
            let path = option_env!("AKULA_INSTALLED_HOST_WITNESS_PATH")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute() && path.file_name().is_some())
                .ok_or_else(|| refused(
                    "host custody witness HOLD: compiler-pinned install witness missing".into()
                ))?;
            return SharedHostCustodyRegistry::open_existing_installer_witness(db, &path)
                .map_err(|error| refused(format!(
                    "host custody witness HOLD: install witness or SQLite identity invalid: {error:?}"
                )));
        }
        #[cfg(not(feature = "sync-installed-host-witness"))]
        SharedHostCustodyRegistry::open_existing(db).map_err(|error| refused(format!(
            "shared custody registry unavailable: {error:?}"
        )))
    }

    /// A verified S3 editor can enter host-wide custody ONLY by persisting
    /// the same root/profile/VaultId/OS-lock identity under the shared SQLite
    /// enrollment transaction. The caller must already hold the process
    /// publication + File Gate + HistoryService enrollment fences.
    ///
    /// The shared entry is committed BEFORE the private DocumentHub marker:
    /// a failed private insert leaves the machine-wide HOLD in place. This
    /// is a one-way safety transition, never an S3 or local write capability.
    #[cfg(feature = "sync-s3")]
    pub(crate) fn enroll_installed_host_s3_participant(
        &self,
        record: &crate::sync::host_custody_registry::HostCustodyRecord,
    ) -> Result<(), FileCommandError> {
        if !self.installed_host_required {
            return Err(FileCommandError::Task {
                message: "host S3 enrollment requires the mandatory installed-host policy".into(),
            });
        }
        // A root-owned witness proves a receipt namespace, NOT integrity of
        // same-UID-writable SQLite rows. Until a privileged broker owns the
        // SQLite writer epoch, even verified receipts grant NO S3 enrollment.
        #[cfg(feature = "sync-installed-host-unix-root-witness")]
        if self.installed_host_required {
            return Err(FileCommandError::Task {
                message: "host custody enrollment HOLD: privileged SQLite broker not implemented".into(),
            });
        }
        // Keep the stable outward S3 enrollment diagnostic even when the
        // shared opener refuses an unavailable or replaced install witness.
        // The user must still see HOLD rather than a retryable empty roster.
        let mut host = self.open_installed_host_registry()
            .map_err(|error| FileCommandError::Task {
                message: format!("host custody enrollment HOLD: {error:?}"),
            })?;
        host.enroll(record).map_err(|error| FileCommandError::Task {
            message: format!("host custody enrollment HOLD: {error:?}"),
        })?;
        Ok(())
    }



    /// Only the still-held host SQLite enrollment epoch can invoke this
    /// exception, never a normal File Gate entrypoint or a network reader.
    #[cfg(feature = "host-custody")]
    pub(crate) fn file_mutation_guard_for_host_write(
        &self, permit: &HostUnenrolledEpochPermit<'_>,
    ) -> Result<FileMutationGuard<'_>, FileCommandError> {
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
    ) -> Result<FileMutationGuard<'_>, FileCommandError> {
        let guard = self.file_mutation_guard()?;
        #[cfg(feature = "host-custody")]
        if !proven_custody && !host_epoch_held && self.installed_host_required {
            return Err(FileCommandError::Task {
                message: "host custody is HOLD: mutation has no held shared enrollment epoch".into(),
            });
        }
        #[cfg(not(feature = "host-custody"))]
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
    #[cfg(feature = "host-custody")]
    pub(crate) fn with_unenrolled_host_epoch<T>(
        &self,
        target: &Path,
        operation: impl FnOnce(&HostUnenrolledEpochPermit<'_>) -> Result<T, FileCommandError>,
    ) -> Result<T, FileCommandError> {
        use crate::sync::filegate_lock_probe::PinnedProbeRoot;
        use crate::sync::host_custody_registry::PhysicalObjectId;
        use std::fs;

        let refused = |message: String| FileCommandError::Task { message };

        // Owner/mode verification of the independent receipt alone does NOT
        // prevent a same-user process from editing SQLite rows in place.
        // Keep ALL ordinary protected writers on HOLD until the actual
        // installer-controlled SQLite broker mediates the shared epoch.
        #[cfg(feature = "sync-installed-host-unix-root-witness")]
        if self.installed_host_required {
            return Err(refused(
                "host custody HOLD: privileged SQLite broker not implemented".into()
            ));
        }

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
            // ui_state may retain a normal Windows C:\\... spelling while
            // the pinned caller supplies fs::canonicalize's \\\\?\\C:\\...
            // spelling (and macOS /var may resolve to /private/var).
            // Require a LEXICAL prefix under one of those two names as well
            // as the independently resolved physical target: a symlink from
            // elsewhere into the root is never accepted solely by resolving.
            let canonical = fs::canonicalize(&root)?;
            let relative = target.strip_prefix(&root)
                .or_else(|_| target.strip_prefix(&canonical));
            if let Ok(relative) = relative {
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
        let mut registry = self.open_installed_host_registry()?;
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

#[cfg(all(test, feature = "sync-installed-host"))]
#[path = "app_core_installed_host_tests.rs"]
mod installed_host_tests;
