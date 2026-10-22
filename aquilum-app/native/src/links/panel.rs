#[cfg(feature = "extension-view-lab")]
mod task_view_lab;

use std::path::{Path, PathBuf};

use aquilum_core::Core;
use aquilum_core::search::analysis::models::AnalysisMethod;
use aquilum_core::wikixiv::models::SearchRequest;

use super::{Item, Listing, Method, Mode, Request, Target};
use crate::i18n::t;

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn run(core: &Core, request: &Request) -> Listing {
    let (workspace, document) = (path_str(&request.workspace), path_str(&request.document));
    let listing = |items: Vec<Item>, empty: &str| Listing {
        generation: request.generation,
        notice: items.is_empty().then(|| t(empty)),
        items,
    };
    let notice = |key: &str| Listing { generation: request.generation, items: Vec::new(), notice: Some(t(key)) };
    match (request.mode, request.method) {
        (Mode::History, _) => listing(Vec::new(), "history.empty"),
            #[cfg(feature = "extension-view-lab")]
        (Mode::Tasks, _) => task_view_lab::run(core, request),
        (Mode::Backlinks, _) => {
            let items = core.search.backlinks(&workspace, &document).unwrap_or_default();
            let items = counted(items.into_iter().map(|link| (link.title, Target::Note(PathBuf::from(link.path), Some(link.offset)))));
            listing(items, "backlinks.empty")
        }
        (Mode::Outgoing, _) => {
            let items = core.search.outgoing_links(&workspace, &document).unwrap_or_default();
            let items = counted(items.into_iter().map(|link| {
                let target = link.path.map_or(Target::Wiki(link.target), |p| Target::Note(PathBuf::from(p), None));
                (link.title, target)
            }));
            listing(items, "backlinks.outgoingEmpty")
        }
        (Mode::Analysis, Method::Wikixiv) => {
            let enabled = core.settings.get_config().analysis.enable_wikixiv;
            let search = SearchRequest { text: request.text.clone(), document_path: Some(document), generation: request.generation };
            match core.wikixiv.search(search, enabled) {
                Ok(result) if result.offline => notice("wikixiv.offline"),
                Ok(result) if result.insufficient_text => notice("wikixiv.insufficient"),
                Ok(result) => {
                    let items = result.hits.into_iter().map(|hit| Item { title: hit.title, counter: None, target: Target::Url(hit.url), reasons: hit.matched_terms }).collect();
                    listing(items, "wikixiv.empty")
                }
                Err(_) => notice("wikixiv.error"),
            }
        }
        (Mode::Analysis, method) => {
            let method = if method == Method::AdamicAdar { AnalysisMethod::AdamicAdar } else { AnalysisMethod::Bm25f };
            let config = core.settings.get_config();
            match core.search.analyze_document(&workspace, &document, method, None, &config) {
                Ok(results) => {
                    let items = results
                        .into_iter()
                        .map(|r| Item {
                            counter: Some(match r.confidence {
                                Some(confidence) => format!("{:.1}%", confidence * 100.0),
                                None => format!("{:.2}", r.raw_score),
                            }),
                            title: r.title,
                            target: Target::Note(PathBuf::from(r.path), None),
                            reasons: r.reasons,
                        })
                        .collect();
                    listing(items, "analysis.empty")
                }
                Err(_) => notice("analysis.error"),
            }
        }
    }
}

fn counted(links: impl Iterator<Item = (String, Target)>) -> Vec<Item> {
    let mut items: Vec<(Item, usize)> = Vec::new();
    for (title, target) in links {
        let key = |target: &Target| match target {
            Target::Wiki(name) => Target::Wiki(name.to_lowercase()),
            Target::Note(path, _) => Target::Note(path.clone(), None),
            other => other.clone(),
        };
        let wanted = key(&target);
        let same = |item: &Item| key(&item.target) == wanted;
        match items.iter_mut().find(|(item, _)| same(item)) {
            Some((_, count)) => *count += 1,
            None => items.push((Item { title, counter: None, target, reasons: Vec::new() }, 1)),
        }
    }
    items
        .into_iter()
        .map(|(mut item, count)| {
            item.counter = Some(count.to_string());
            item
        })
        .collect()
}

#[cfg(test)]
mod tests;
