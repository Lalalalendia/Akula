use crate::search::paths::is_markdown;
use notify::event::{ModifyKind, RemoveKind};
use notify::{Event, EventKind};

pub(super) fn is_structural(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Remove(_) | EventKind::Modify(ModifyKind::Name(_))
    )
}

pub(super) fn needs_directory_rescan(event: &Event) -> bool {
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

