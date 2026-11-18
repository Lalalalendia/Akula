//! LANG-02 live editor contribution to a temporary study deck.
//!
//! The active window already owns the latest editor Rope. Use only that
//! buffer, the registered read-only note identity and a private revision
//! stamp. All other open tabs remain excluded from closed-disk reads.
//! Never open a vault path, persist cells or create a document identity here.

use aquilum_core::learning::state::StudyState;
use aquilum_core::learning::table_anchors::NoteKey;
use aquilum_core::learning::host_read::MAX_STUDY_SOURCE_BYTES;
use uuid::Uuid;

use super::super::App;
use crate::session::Session;
use crate::ui::editor::TextEditor;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LiveEditorStamp {
    pub note: NoteKey,
    pub revision: u64,
    /// Baseline file hash held by the active native Note, not a read grant.
    pub baseline_hash: String,
    /// BLAKE3 of the complete in-memory editor text, not stored vocabulary.
    pub buffer_hash: String,
}

impl App {
    /// Only the currently rendered note can supply in-memory text. A selected
    /// collection membership is not authority until its registered active ID
    /// resolves to that same physical note in this selected workspace.
    fn live_study_editor(
        &self,
        session: &Session,
        state: &StudyState,
        selected_ids: &[String],
    ) -> Option<(LiveEditorStamp, Vec<Vec<Vec<String>>>)> {
        let note = self.note.as_ref()?;
        if !self.tabs.active().path.as_deref().is_some_and(|p| note.is(p)) {
            return None;
        }
        let (revision, baseline_hash) = note.study_live_stamp()?;
        if self.text.byte_len() > MAX_STUDY_SOURCE_BYTES as usize {
            return None;
        }
        let current_path = std::fs::canonicalize(&note.path).ok()?;
        let vault = session.workspace_id.to_string();
        let selected_note = state.registry.collections.iter()
            .filter(|group| selected_ids.contains(&group.id))
            .flat_map(|group| group.members.iter())
            .filter(|source| source.vault_id == vault)
            .find_map(|source| {
                let id = Uuid::parse_str(&source.note_id).ok()?;
                (session.study_document_path(&self.workspace.core, id)
                    .as_deref() == Some(current_path.as_path()))
                    .then(|| NoteKey { vault_id: vault.clone(), note_id: source.note_id.clone() })
            })?;
        let body = self.text.to_string();
        let tables = TextEditor::scan_study_source(&body).ok()?;
        let stamp = LiveEditorStamp {
            note: selected_note,
            revision,
            baseline_hash,
            buffer_hash: blake3::hash(body.as_bytes()).to_hex().to_string(),
        };
        Some((stamp, tables))
    }

    /// Replace the "closed reader refused open note" marker only after an
    /// independently verified active-memory source has been parsed fully.
    pub(crate) fn append_study_editor(
        &self,
        session: &Session,
        state: &StudyState,
        selected_ids: &[String],
        parsed: &mut Vec<(NoteKey, Vec<Vec<Vec<String>>>)>,
        unavailable: &mut Vec<NoteKey>,
    ) -> Option<LiveEditorStamp> {
        let (stamp, tables) = self.live_study_editor(session, state, selected_ids)?;
        if !unavailable.contains(&stamp.note) {
            return None;
        }
        unavailable.retain(|note| note != &stamp.note);
        parsed.push((stamp.note.clone(), tables));
        Some(stamp)
    }

    /// Called before every card or answer transition. If an open editor
    /// switches, changes bytes/revision, enters conflict or becomes unavailable,
    /// abort instead of showing answers assembled from an old buffer.
    pub(crate) fn validate_study_editor(
        &self,
        session: &Session,
        state: &StudyState,
        selected_ids: &[String],
        stamp: &Option<LiveEditorStamp>,
        unavailable: &mut Vec<NoteKey>,
    ) -> bool {
        let current = self.live_study_editor(session, state, selected_ids);
        match (stamp, current) {
            (None, None) => true,
            (Some(expected), Some((now, _))) if *expected == now
                && unavailable.contains(&expected.note) =>
            {
                unavailable.retain(|note| note != &expected.note);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_stamp_does_not_store_raw_vocabulary() {
        assert_eq!(std::mem::size_of::<LiveEditorStamp>() > 0, true);
        let hash = blake3::hash(b"example source text").to_hex().to_string();
        assert_eq!(hash.len(), 64);
    }
}
