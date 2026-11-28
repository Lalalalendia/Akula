use crate::{
    app::{App, Outcome, Popup},
    i18n::t,
    ui::{search::CardData, settings::transcript::Editor},
};
use aquilum_core::{
    Core,
    background_jobs::{
        Queue,
        speech::search::{Hit, Session, Target},
    },
    text::{cancel::Cancel, matcher::Matcher},
};
use masonry::{
    app::RenderRoot,
    core::{ErasedAction, WidgetId},
};
use std::path::Path;

#[derive(Default)]
pub(super) struct Search {
    queue: Option<Queue>,
    session: Session,
}

impl Search {
    pub(super) fn find(
        &mut self,
        core: &Core,
        profile: &Path,
        workspace: Option<&Path>,
        query: &str,
        cancel: &Cancel,
    ) -> Vec<Hit> {
        if cancel.is_cancelled() {
            return Vec::new();
        }
        if self.queue.is_none() {
            let database = profile.join("media-jobs.sqlite");
            if database.is_file() {
                self.queue = Queue::open(&database).ok();
            }
        }
        let Some(queue) = &self.queue else {
            return Vec::new();
        };
        let mut roots = core.known_roots();
        let vault = workspace.and_then(|path| path.canonicalize().ok());
        if let Some(vault) = &vault {
            roots.push(vault.clone());
        }
        let library = crate::video::library::search_root(profile, &roots);
        self.session
            .search(queue, query, vault.as_deref(), library.as_deref(), cancel)
            .unwrap_or_default()
    }
}

pub(super) fn card(hit: &Hit, matcher: &Matcher) -> CardData {
    let seconds = hit.start_ms / 1000;
    CardData {
        title: hit
            .path
            .file_name()
            .unwrap_or(hit.path.as_os_str())
            .to_string_lossy()
            .into_owned(),
        extension: t("search.transcript"),
        snippet: hit.snippet.clone(),
        count: format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        ),
        title_marks: Vec::new(),
        extension_marks: Vec::new(),
        snippet_marks: matcher
            .find_all(&hit.snippet)
            .into_iter()
            .map(|found| (found.start, found.end))
            .collect(),
    }
}

fn marks(matcher: &Matcher, text: &str) -> Vec<(usize, usize)> {
    matcher
        .find_all(text)
        .into_iter()
        .map(|found| (found.start, found.end))
        .collect()
}

pub(super) fn cards(
    state: &super::SearchState,
) -> Vec<masonry::core::NewWidget<crate::ui::search::ResultCard>> {
    use crate::{i18n::plural, ui::search::ResultCard};
    use aquilum_core::search::hits::query_matcher;
    let matcher = query_matcher(&state.terms);
    let cue_matcher = query_matcher(&aquilum_core::background_jobs::speech::search::terms(
        &state.query,
    ));
    state
        .results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            let data = CardData {
                title_marks: marks(&matcher, &result.title),
                extension_marks: marks(&matcher, &result.extension),
                snippet_marks: marks(&matcher, &result.snippet),
                title: result.title.clone(),
                extension: result.extension.clone(),
                snippet: result.snippet.clone(),
                count: plural("search.matches", result.match_count as u64),
            };
            ResultCard::new(data, index == state.selected)
        })
        .chain(state.speech.iter().enumerate().map(|(index, hit)| {
            ResultCard::new(
                card(hit, &cue_matcher),
                state.results.len() + index == state.selected,
            )
        }))
        .collect()
}

impl App {
    pub(super) fn open_selected(&mut self, root: &mut RenderRoot, new_tab: bool) -> Outcome {
        let Some(Popup::Search(state)) = &self.popup else {
            return Outcome::default();
        };
        if let Some(hit) = state
            .selected
            .checked_sub(state.results.len())
            .and_then(|index| state.speech.get(index))
        {
            let target = hit.target;
            self.close_search(root);
            self.open_search_transcript(root, target);
            return Outcome::default();
        }
        let Some(result) = state.results.get(state.selected) else {
            return Outcome::default();
        };
        let path = std::path::PathBuf::from(&result.path);
        let offset = result.match_offset;
        self.close_search(root);
        let title = self.open_in_with_offset(root, path, new_tab, Some(offset));
        Outcome {
            title,
            ..Outcome::default()
        }
    }

    pub(super) fn open_search_transcript(&mut self, root: &mut RenderRoot, target: Target) {
        let (layer, mut editor) = Editor::from_search(self.workspace.speech(), target);
        crate::app::ui_actions::mount_modal(root, layer, editor.layer());
        editor.poll(root);
        self.popup = Some(Popup::Transcript(Box::new(editor)));
    }

    pub(in crate::app) fn on_search_transcript_action(
        &mut self,
        root: &mut RenderRoot,
        id: WidgetId,
        action: &ErasedAction,
    ) -> Outcome {
        if let Some(Popup::Transcript(mut editor)) = self.popup.take() {
            if editor.action(root, self.workspace.speech(), id, action) {
                root.remove_layer(editor.layer());
                root.focus_on(self.editor_id());
            } else {
                editor.poll(root);
                self.popup = Some(Popup::Transcript(editor));
            }
        }
        Outcome::default()
    }
}

impl super::SearchState {
    pub(super) fn apply(&mut self, results: super::worker::Results) {
        use super::worker::Results;
        match results {
            Results::Speech(hits) => self.speech = hits,
            Results::Notes(response) => {
                self.speech.clear();
                self.selected = 0;
                self.failed = response.is_err();
                match response {
                    Ok(response) => {
                        self.results = response.results;
                        self.terms = response.query_terms;
                        self.status = Some(response.status);
                    }
                    Err(error) => {
                        self.results.clear();
                        report!("поиск не выполнен: {error}");
                    }
                }
            }
        }
    }
}
