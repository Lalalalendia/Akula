//! LANG-02j: bounded first-party reader for selected *closed* study notes.
//!
//! This is not a plugin import, a file permission grant, or a way to open an
//! arbitrary path. The caller is trusted native host code operating on its
//! live selected workspace. Stored source IDs only select registered documents.
//!
//! All bytes come through the registered pinned OS reader and DocumentHub
//! closed-document reservation, never std::fs::read(path). Open/dirty editor
//! sessions, symlinks, hardlinks, changed file identities, invalid paths and
//! unregistered roots fail closed. A caller must revalidate the returned
//! digest before presenting a card or applying any answer/row action.

use std::collections::BTreeSet;
use std::path::Path;
use uuid::Uuid;

use crate::app_core::Core;
use crate::files::document::text_from_bytes;
use crate::learning::state::{self, StateError, StudyState};
use crate::learning::table_anchors::NoteKey;
use crate::sync::{RegisteredReadAdmission, SyncRelativePath};

pub const MAX_STUDY_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_STUDY_SOURCE_NOTES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StudyReadError {
    Admission,
    InvalidState(StateError),
    UnknownCollection,
    TooManySources,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveStudyText {
    pub note: NoteKey,
    /// Normalized text from one completed descriptor-relative read.
    pub text: String,
    /// BLAKE3 of ORIGINAL file bytes, not normalized text or durable authority.
    pub raw_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedStudyTexts {
    pub available: Vec<LiveStudyText>,
    /// Only opaque IDs, never old stored cell text.
    pub unavailable: Vec<NoteKey>,
}

/// The selected workspace and its registered identity come from the native
/// session. This object does not accept guest-provided roots or note paths.
pub struct HostStudyReader<'a> {
    core: &'a Core,
    admission: RegisteredReadAdmission<'a>,
    workspace_id: Uuid,
}

impl<'a> HostStudyReader<'a> {
    pub fn open(
        core: &'a Core,
        selected_root: &Path,
        expected_workspace_id: Uuid,
    ) -> Result<Self, StudyReadError> {
        let admission = RegisteredReadAdmission::open(core, selected_root)
            .map_err(|_| StudyReadError::Admission)?;
        if admission.registered_workspace_id() != expected_workspace_id {
            return Err(StudyReadError::Admission);
        }
        Ok(Self { core, admission, workspace_id: expected_workspace_id })
    }

    /// Return only sources from explicitly selected collection IDs.
    /// A failed per-note read becomes an unavailable ID, not a partial leak.
    /// No Markdown, vocabulary or card text is written to sidecars.
    pub fn read_selected(
        &self,
        state: &StudyState,
        selected_ids: &[String],
    ) -> Result<SelectedStudyTexts, StudyReadError> {
        self.read_selected_excluding(state, selected_ids, &BTreeSet::new())
    }

    /// Trusted caller must exclude every document with an open native editor
    /// tab, even when DocumentHub itself has no open session for that tab.
    /// Exclusions are evaluated BEFORE any descriptor or file content read.
    pub fn read_selected_excluding(
        &self,
        state: &StudyState,
        selected_ids: &[String],
        excluded: &BTreeSet<NoteKey>,
    ) -> Result<SelectedStudyTexts, StudyReadError> {
        state::validate(state).map_err(StudyReadError::InvalidState)?;
        let mut notes = BTreeSet::new();
        for selected_id in selected_ids {
            let group = state.registry.collections.iter()
                .find(|group| group.id == *selected_id)
                .ok_or(StudyReadError::UnknownCollection)?;
            for source in &group.members {
                notes.insert(NoteKey {
                    vault_id: source.vault_id.clone(),
                    note_id: source.note_id.clone(),
                });
            }
        }
        if notes.len() > MAX_STUDY_SOURCE_NOTES {
            return Err(StudyReadError::TooManySources);
        }
        let mut result = SelectedStudyTexts {
            available: Vec::new(), unavailable: Vec::new(),
        };
        for note in notes {
            if excluded.contains(&note) {
                result.unavailable.push(note);
                continue;
            }
            match self.read_closed(&note) {
                Some((text, raw_hash)) => result.available.push(
                    LiveStudyText { note, text, raw_hash },
                ),
                None => result.unavailable.push(note),
            }
        }
        Ok(result)
    }

    /// A digest or opaque ID is not enough to present an old cached card.
    /// Recheck the current registered note using the same descriptor-only
    /// admission. The native host must also verify current UI permission.
    pub fn unchanged(&self, snapshot: &LiveStudyText) -> bool {
        self.read_closed(&snapshot.note)
            .is_some_and(|(text, hash)| hash == snapshot.raw_hash && text == snapshot.text)
    }

    fn read_closed(&self, note: &NoteKey) -> Option<(String, String)> {
        if note.vault_id != self.workspace_id.to_string() {
            return None;
        }
        let document = Uuid::parse_str(&note.note_id).ok()?;
        let path = self.core.ui_state
            .active_document_relative(self.workspace_id, document).ok()??;
        let relative = SyncRelativePath::parse(&path).ok()?;
        if relative.as_str() != path
            || !Path::new(&path).extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
        {
            return None;
        }
        let file_id = self.admission.bind_plugin_file_identity(&relative).ok()?;
        let read = self.admission.read_plugin_identified_bounded(
            &relative, file_id, MAX_STUDY_SOURCE_BYTES,
        ).ok()?;
        // The document may be marked missing/renamed while the pinned read is
        // happening. A new document at the same path cannot reuse this ID.
        let rechecked = self.core.ui_state
            .active_document_relative(self.workspace_id, document).ok()??;
        if rechecked != path || self.admission.recheck_registered_root().is_err() {
            return None;
        }
        let content = text_from_bytes(read.bytes).ok()?;
        Some((content, read.content_hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::{ColumnMapping, state::bind_table};
    use std::sync::Arc;

    fn table(word: &str, answer: &str) -> Vec<Vec<String>> {
        vec![
            vec!["Q".into(), "A".into()],
            vec![word.into(), answer.into()],
        ]
    }

    struct Case {
        core: Arc<Core>,
        vault: tempfile::TempDir,
        _data: tempfile::TempDir,
        workspace: Uuid,
        state: StudyState,
        path: std::path::PathBuf,
        doc: Uuid,
    }

    fn fixture() -> Case {
        let vault = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let path = vault.path().join("words.md");
        std::fs::write(&path, "| Q | A |\n| --- | --- |\n| eat | есть |").unwrap();
        let core = Core::open(data.path(), Arc::new(|_| {}));
        let workspace = core.ui_state
            .resolve_workspace(&vault.path().to_string_lossy(), 1).unwrap();
        let doc = core.ui_state.resolve_document(workspace, "words.md").unwrap();
        let key = NoteKey { vault_id: workspace.to_string(), note_id: doc.to_string() };
        let rows = [table("eat", "есть")];
        let mut state = StudyState::default();
        bind_table(&mut state, &key, &rows, 0,
            ColumnMapping { question: 0, answers: vec![1], context: None },
            "words", "Words", "t1").unwrap();
        Case { core, vault, _data: data, workspace, state, path, doc }
    }

    #[test]
    fn registered_closed_note_is_read_only_and_rechecks_current_hash() {
        let case = fixture();
        let original = std::fs::read(&case.path).unwrap();
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        let sources = reader.read_selected(&case.state, &["words".into()]).unwrap();
        assert!(sources.unavailable.is_empty());
        assert_eq!(sources.available.len(), 1);
        assert!(sources.available[0].text.contains("eat | есть"));
        assert!(reader.unchanged(&sources.available[0]));
        assert_eq!(std::fs::read(&case.path).unwrap(), original);
        std::fs::write(&case.path, "| Q | A |\n| --- | --- |\n| eat | другой |").unwrap();
        assert!(!reader.unchanged(&sources.available[0]));
    }

    #[test]
    fn closed_reader_rejects_other_workspace_or_missing_document() {
        let case = fixture();
        assert!(matches!(
            HostStudyReader::open(&case.core, case.vault.path(), Uuid::new_v4()),
            Err(StudyReadError::Admission)
        ));
        case.core.ui_state.mark_document_missing(case.doc, 44).unwrap();
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        let output = reader.read_selected(&case.state, &["words".into()]).unwrap();
        assert!(output.available.is_empty());
        assert_eq!(output.unavailable.len(), 1);
    }

    #[test]
    fn open_editor_blocks_closed_source_read_instead_of_leaking_stale_disk() {
        let case = fixture();
        let editor = case.core.documents.open(&case.core, &case.path, 0).unwrap();
        assert!(editor.text.contains("eat | есть"), "the editor must actually be open");
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        let output = reader.read_selected(&case.state, &["words".into()]).unwrap();
        assert!(output.available.is_empty());
        assert_eq!(output.unavailable.len(), 1);
    }

    #[test]
    fn invalid_collection_is_not_an_implicit_permission_grant() {
        let case = fixture();
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        assert_eq!(
            reader.read_selected(&case.state, &["unknown".into()]),
            Err(StudyReadError::UnknownCollection),
        );
        let mut changed = case.state;
        changed.registry.collections[0].members[0].note_id = Uuid::new_v4().to_string();
        assert!(matches!(
            reader.read_selected(&changed, &["words".into()]),
            Err(StudyReadError::InvalidState(_)),
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_source_is_unavailable_even_when_registered() {
        use std::os::unix::fs::symlink;
        let case = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::remove_file(&case.path).unwrap();
        let secret = outside.path().join("secret.md");
        std::fs::write(&secret, "| Q | A |\n| --- | --- |\n| eat | secret |").unwrap();
        symlink(&secret, &case.path).unwrap();
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        let output = reader.read_selected(&case.state, &["words".into()]).unwrap();
        assert!(output.available.is_empty());
        assert_eq!(output.unavailable.len(), 1);
        assert!(!output.available.iter().any(|note| note.text.contains("secret")));
    }

    #[test]
    fn caller_exclusion_refuses_open_native_tab_before_reading_file() {
        let case = fixture();
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        let key = NoteKey { vault_id: case.workspace.to_string(), note_id: case.doc.to_string() };
        let excluded = BTreeSet::from([key.clone()]);
        let output = reader.read_selected_excluding(
            &case.state, &["words".into()], &excluded,
        ).unwrap();
        assert!(output.available.is_empty());
        assert_eq!(output.unavailable, [key]);
        // Exclusion is not a mutation of original notes or saved membership.
        assert!(case.path.is_file());
    }

    #[test]
    fn requested_notes_are_deduplicated_before_any_read() {
        let mut case = fixture();
        let key = NoteKey { vault_id: case.workspace.to_string(), note_id: case.doc.to_string() };
        bind_table(&mut case.state, &key, &[table("eat", "есть")], 0,
            ColumnMapping { question: 0, answers: vec![1], context: None },
            "verbs", "Verbs", "unused").unwrap();
        let reader = HostStudyReader::open(&case.core, case.vault.path(), case.workspace).unwrap();
        let output = reader.read_selected(&case.state, &["words".into(), "verbs".into()]).unwrap();
        assert_eq!(output.available.len(), 1);
        assert!(output.unavailable.is_empty());
    }
}
