mod event_kinds;
mod qa_import;

use super::batch::{WatchBatch, WatchScope};
use crate::search::paths::{is_hidden, is_markdown, is_plain_text};
use notify::{Event, EventKind};
use std::path::Path;

pub fn classify(result: notify::Result<Event>, root: &Path) -> Option<WatchBatch> {
    let Ok(event) = result else {
        return Some(WatchBatch::rescan());
    };
    if matches!(event.kind, EventKind::Access(_)) {
        return None;
    }
    // Internal history, trash and sync-stage writes must not reach the index.
    let paths = event.paths.into_iter().filter(|path| !is_hidden(root, path)).collect::<Vec<_>>();
    if paths.is_empty() {
        return None;
    }
    let event = Event { paths, ..event };
    if event_kinds::needs_directory_rescan(&event) {
        return Some(WatchBatch::new(event.paths, WatchScope::Rescan));
    }

    let scope = if event_kinds::is_structural(&event.kind) {
        WatchScope::Structure
    } else {
        WatchScope::Content
    };
    let notes = event
        .paths
        .iter()
        .filter(|path| is_markdown(path) || is_plain_text(path) || qa_import::is_qa_import_copy(path))
        .cloned()
        .collect::<Vec<_>>();
    if !notes.is_empty() {
        return Some(WatchBatch::new(notes, scope));
    }
    if scope != WatchScope::Structure {
        return None;
    }
    let listed = event
        .paths
        .into_iter()
        .filter(|path| !is_hidden(root, path))
        .collect::<Vec<_>>();
    (!listed.is_empty()).then(|| WatchBatch::new(listed, WatchScope::Listing))
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
