mod moves;
mod trash;
mod write;

#[cfg(test)]
mod tests;

pub use moves::{move_across, rename};
pub use trash::{cleanup_retention, ensure_trash, restore, restore_deletion, trash};
pub use write::{copy, create, create_binary, create_folder, ensure_folder_tree, name_version, write};

use super::attachments::forget_attachments_if_touched;
use crate::app_core::{Core, CoreEvent};
use crate::history::NoteWrite;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Relocation {
    pub moves: Vec<NoteMove>,
    pub removed: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct NoteHistoryChanged {
    pub path: String,
}

/// The note was committed, but its history snapshot or label did not persist.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteHistorySaveFailed {
    pub path: String,
    pub action: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NoteMove {
    pub from: String,
    pub to: String,
}

fn record(core: &Core, note: &Path, write: NoteWrite<'_>) {
    match core.history.try_saved(&|| core.known_roots(), note, write) {
        Ok(true) => announce_history(core, note),
        Ok(false) => {}
        Err(error) => {
            eprintln!("[aquilum:history] {}: {error}", note.display());
            core.emit(CoreEvent::NoteHistorySaveFailed(NoteHistorySaveFailed {
                path: note.to_string_lossy().into_owned(),
                action: error.action.to_owned(),
                message: error.message,
            }));
        }
    }
}

fn paths_changed(core: &Core, paths: Vec<PathBuf>) {
    forget_attachments_if_touched(&paths);
    core.search.notify_paths(paths);
}

fn announce(core: &Core, relocation: Relocation) {
    if relocation.moves.is_empty() && relocation.removed.is_empty() {
        return;
    }
    let moves: Vec<(String, String)> =
        relocation.moves.iter().map(|step| (step.from.clone(), step.to.clone())).collect();
    core.documents.relocated(&moves, &relocation.removed);
    core.emit(CoreEvent::NotesRelocated(relocation));
}

fn announce_history(core: &Core, note: &Path) {
    let changed = NoteHistoryChanged {
        path: note.to_string_lossy().into_owned(),
    };
    core.emit(CoreEvent::NoteHistoryChanged(changed));
}
