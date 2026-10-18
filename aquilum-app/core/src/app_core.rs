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

// Conservatively serialize all Core-managed mutations and protected original
// captures within this process, even when two distinct Core instances use
// the same workspace. External processes still require an OS-level guard.
static PROCESS_FILE_MUTATIONS: Mutex<()> = Mutex::new(());

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
    pub wikixiv: WikixivService,
    pub watcher: WorkspaceWatcher,
    /// The note open in the UI, reported by the host; MCP tools use it for «эта заметка».
    pub active_note: ActiveNote,
    pub mcp: McpServer,
    events: Arc<dyn EventSink>,
}

impl Core {
    /// Opens all stores under `app_data_dir`, which the host picks.
    pub fn open(app_data_dir: &Path, events: Arc<dyn EventSink>) -> Arc<Self> {
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
                settings: SettingsManager::new(app_data_dir),
                ui_state: UiStateService::open(&app_data_dir.join("ui-state.sqlite3")),
                search: SearchService::new(app_data_dir.join("search-v2"), notifier, index_notifier),
                history: HistoryService::new(app_data_dir),
                documents: DocumentHub::new(&app_data_dir.join("documents.sqlite3")),
                file_mutations: &PROCESS_FILE_MUTATIONS,
                #[cfg(feature = "sync-s3")]
                guarded_editors: Mutex::new(Vec::new()),
                wikixiv: WikixivService::new(),
                watcher: WorkspaceWatcher::new(watch_sink),
                active_note: ActiveNote::default(),
                mcp: McpServer::new(),
                events,
            }
        });
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

    /// Mutating File Gate entrypoints must refuse a pending or invalid history
    /// transfer BEFORE changing the primary bytes. This deliberately checks
    /// every registered workspace, mirroring HistoryService's conservative
    /// process-wide recovery hold. The read-only File Gate admission path still
    /// uses file_mutation_guard() so forensic inspection remains possible.
    ///
    /// This is a pathname/SQLite HOLD check under an in-process mutex, NOT
    /// an OS interprocess lock or permission for a sync overwrite/delete.
    pub(crate) fn file_mutation_guard_for_write(&self) -> Result<MutexGuard<'_, ()>, FileCommandError> {
        let guard = self.file_mutation_guard()?;
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
