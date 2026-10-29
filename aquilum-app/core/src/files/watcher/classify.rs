use super::batch::{WatchBatch, WatchScope};
use crate::search::paths::{is_hidden, is_markdown, is_plain_text};
use notify::event::{ModifyKind, RemoveKind};
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
    if needs_directory_rescan(&event) {
        return Some(WatchBatch::new(event.paths, WatchScope::Rescan));
    }

    let scope = if is_structural(&event.kind) {
        WatchScope::Structure
    } else {
        WatchScope::Content
    };
    let notes = event
        .paths
        .iter()
        .filter(|path| is_markdown(path) || is_plain_text(path) || is_qa_import_copy(path))
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

/// Only opt-in P2P may observe content changes of deterministic QA Vault copies.
/// This is an invalidation event, not indexing or File Gate authorization.
#[cfg(feature = "p2p-inbox-lab")]
fn is_qa_import_copy(path: &Path) -> bool {
    path.file_name()
        .and_then(|file| file.to_str())
        .and_then(|file| file.strip_prefix("received-"))
        .and_then(|file| file.strip_suffix(".bin"))
        .is_some_and(|identity| uuid::Uuid::parse_str(identity).is_ok())
}

#[cfg(not(feature = "p2p-inbox-lab"))]
fn is_qa_import_copy(_path: &Path) -> bool {
    false
}

fn is_structural(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    )
}

fn needs_directory_rescan(event: &Event) -> bool {
    match event.kind {
        EventKind::Remove(RemoveKind::Folder) => true,
        EventKind::Remove(_) => event
            .paths
            .iter()
            .any(|path| !is_markdown(path) && path.extension().is_none()),
        EventKind::Modify(ModifyKind::Name(_)) => event.paths.iter().any(|path| path.is_dir()),
        _ => false,
    }
}

#[cfg(test)]
#[path = "classify_tests.rs"]
mod tests;
