//! LANG-02: a fail-closed study deck from closed notes and the active editor.
//! No persisted cells, direct path reads or plugin grants.
//!
//! The active editor uses its in-memory revision; inactive tabs stay unavailable.
//! Revalidate before presenting a card or answer.

use std::collections::BTreeSet;
use aquilum_core::learning::activation::LiveNote;
use aquilum_core::learning::collections::Limits;
use aquilum_core::learning::host_read::{HostStudyReader, StudyReadError};
use aquilum_core::learning::practice::{self, PracticeDeck, PracticeError};
use aquilum_core::learning::state::{StateError, StudyStore};
use aquilum_core::learning::table_anchors::NoteKey;
use super::super::App;
use crate::ui::editor::TextEditor;
use super::study_live::LiveEditorStamp;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StudyLoadError {
    NoWorkspace,
    Store(StateError),
    Read(StudyReadError),
    Project(PracticeError),
    Changed,
}

pub(crate) struct StudyDeckSnapshot {
    pub deck: PracticeDeck,
    pub collection_ids: Vec<String>,
    pub revision: u64,
    /// File digest is a short-lived staleness check, not authorization or
    /// durable review history. Raw text is dropped before this result returns.
    pub sources: Vec<(NoteKey, String)>,
    pub unavailable_notes: Vec<NoteKey>,
    pub active: Option<LiveEditorStamp>,
}
/// Exclude all native tabs before admitting the selected active memory buffer.
fn open_editor_sources(
    app: &App,
    session: &crate::session::Session,
    state: &aquilum_core::learning::state::StudyState,
) -> BTreeSet<NoteKey> {
    // Resolve only indexed paths; Session::document would create unseen IDs.
    let mut open_paths = app.tabs.tabs().iter()
        .filter_map(|tab| tab.path.as_ref())
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect::<BTreeSet<_>>();
    if let Some(note) = app.note.as_ref()
        && let Ok(path) = std::fs::canonicalize(&note.path)
    {
        open_paths.insert(path);
    }
    let mut excluded = BTreeSet::new();
    let vault = session.workspace_id.to_string();
    for binding in &state.registry.tables {
        let source = &binding.source;
        if source.vault_id != vault {
            continue;
        }
        let Some(id) = uuid::Uuid::parse_str(&source.note_id).ok() else { continue };
        let Some(path) = session.study_document_path(&app.workspace.core, id) else { continue };
        if open_paths.contains(&path) {
            excluded.insert(NoteKey {
                vault_id: vault.clone(), note_id: source.note_id.clone(),
            });
        }
    }
    excluded
}
impl App {
    /// Construct a temporary combined set across selected collections. All
    /// source text is verified against the exact same atomic metadata revision
    /// and is dropped after creating the deck.
    pub(crate) fn load_study_deck(
        &self,
        collection_ids: &[String],
    ) -> Result<StudyDeckSnapshot, StudyLoadError> {
        let session = self.session.as_ref().ok_or(StudyLoadError::NoWorkspace)?;
        let store = StudyStore::open(&self.workspace.data_dir.join("language-study.sqlite"))
            .map_err(StudyLoadError::Store)?;
        let stored = store.load().map_err(StudyLoadError::Store)?;
        let reader = HostStudyReader::open(
            &self.workspace.core, session.study_root(), session.workspace_id,
        ).map_err(StudyLoadError::Read)?;
        let excluded = open_editor_sources(self, session, &stored.state);
        let batch = reader.read_selected_excluding(&stored.state, collection_ids, &excluded)
            .map_err(StudyLoadError::Read)?;
        let mut parsed = Vec::<(NoteKey, Vec<Vec<Vec<String>>>)>::new();
        let mut source_stamps = Vec::new();
        for source in &batch.available {
            source_stamps.push((source.note.clone(), source.raw_hash.clone()));
            // A malformed or oversized live Markdown source is held by the
            // activation join, not turned into guessed quiz cells.
            if let Ok(tables) = TextEditor::scan_study_source(&source.text) {
                parsed.push((source.note.clone(), tables));
            }
        }
        let mut unavailable_notes = batch.unavailable;
        let active = self.append_study_editor(session, &stored.state, collection_ids,
            &mut parsed, &mut unavailable_notes);
        let live = parsed.iter().map(|(note, tables)| LiveNote {
            note,
            tables,
        }).collect::<Vec<_>>();
        let deck = practice::build_deck(
            &stored.state, collection_ids, &live,
            Limits { max_tables: 256, max_rows_per_table: 512 },
        ).map_err(StudyLoadError::Project)?;
        let snapshot = StudyDeckSnapshot {
            deck,
            collection_ids: collection_ids.to_vec(),
            revision: stored.revision,
            sources: source_stamps,
            unavailable_notes,
            active,
        };
        // New external writes or deleted/renamed sources cannot turn the
        // just-produced deck into an apparently current session.
        if !self.revalidate_study_deck(&snapshot) {
            return Err(StudyLoadError::Changed);
        }
        Ok(snapshot)
    }
    /// Must be called immediately before presenting/using a card. Does not
    /// allow a stale fingerprint or stored path to reopen a revoked document.
    pub(crate) fn revalidate_study_deck(&self, snapshot: &StudyDeckSnapshot) -> bool {
        let Some(session) = self.session.as_ref() else { return false };
        let Ok(store) = StudyStore::open(
            &self.workspace.data_dir.join("language-study.sqlite"),
        ) else { return false };
        let Ok(saved) = store.load() else { return false };
        if saved.revision != snapshot.revision {
            return false;
        }
        let Ok(reader) = HostStudyReader::open(
            &self.workspace.core, session.study_root(), session.workspace_id,
        ) else { return false };
        let excluded = open_editor_sources(self, session, &saved.state);
        let Ok(current) = reader.read_selected_excluding(
            &saved.state, &snapshot.collection_ids, &excluded,
        ) else { return false };
        let mut unavailable = current.unavailable;
        if !self.validate_study_editor(session, &saved.state, &snapshot.collection_ids,
            &snapshot.active, &mut unavailable) || unavailable != snapshot.unavailable_notes {
            return false;
        }
        let now = current.available.iter().map(|source| {
            (source.note.clone(), source.raw_hash.clone())
        }).collect::<Vec<_>>();
        now == snapshot.sources
    }
}
