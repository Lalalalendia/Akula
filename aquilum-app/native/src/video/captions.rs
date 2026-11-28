use super::PLAYERS;
use crate::workspace::speech_jobs::playback::Attached;
use aquilum_core::background_jobs::speech::playback::MediaSource;
use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

thread_local! {
    static SELECTED: RefCell<Option<Arc<Attached>>> = const { RefCell::new(None) };
}

pub(crate) fn select(track: Option<Arc<Attached>>) {
    SELECTED.with(|selected| *selected.borrow_mut() = track);
}

pub(crate) fn text_at(path: &Path, position: f64) -> Option<Arc<str>> {
    PLAYERS.with(|players| for_source(players.borrow().get(path)?.source.as_ref(), position))
}

pub(super) fn for_source(source: Option<&MediaSource>, position: f64) -> Option<Arc<str>> {
    SELECTED.with(|selected| selected.borrow().as_ref()?.text_at(source?, position))
}
