use super::{Ask, Found, Results, SearchResponse};
use aquilum_core::{
    Core,
    search::{error::SearchError, query::Request},
};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

const TYPING_DELAY: Duration = Duration::from_millis(70);

fn latest(inbox: &Receiver<Ask>, mut ask: Ask) -> Option<Ask> {
    loop {
        match inbox.recv_timeout(TYPING_DELAY) {
            Ok(newer) => ask = newer,
            Err(RecvTimeoutError::Timeout) => return Some(ask),
            Err(RecvTimeoutError::Disconnected) => return None,
        }
    }
}

pub(super) fn serve(
    worker: u64,
    core: &Core,
    profile: &std::path::Path,
    inbox: &Receiver<Ask>,
    found: &Sender<Found>,
    wake: &(dyn Fn() + Send + Sync),
) {
    let mut speech_search = super::super::media::Search::default();
    while let Ok(first) = inbox.recv() {
        let Some(ask) = latest(inbox, first) else {
            return;
        };
        let request = Request {
            cancel: ask.cancel.clone(),
            ..Request::new(&ask.query)
        };
        let response = match ask.workspace.as_ref().map_or_else(
            || Ok(SearchResponse::idle()),
            |workspace| core.search.search(&workspace.to_string_lossy(), &request),
        ) {
            Err(SearchError::Cancelled) => continue,
            response => response.map_err(|e| format!("{e:?}")),
        };
        let send = |results| {
            if ask.cancel.is_cancelled() {
                return true;
            }
            if found
                .send(Found {
                    generation: ask.cancel.generation(),
                    worker,
                    results,
                })
                .is_err()
            {
                return false;
            }
            wake();
            true
        };
        if !send(Results::Notes(response)) {
            return;
        }
        let speech = speech_search.find(
            core,
            profile,
            ask.workspace.as_deref(),
            &ask.query,
            &ask.cancel,
        );
        if !speech.is_empty() && !send(Results::Speech(speech)) {
            return;
        }
    }
}
