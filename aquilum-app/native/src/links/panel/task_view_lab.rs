//! First-party read-only task view. No guest API, file access or task writes.
use aquilum_core::Core;

use super::super::{Item, Listing, Request, Target};
use crate::i18n::t;

pub(super) fn run(core: &Core, request: &Request) -> Listing {
    let workspace = request.workspace.to_string_lossy();
    match core.search.task_preview_for_document(&workspace, &request.document) {
        Ok(tasks) => {
            let more = tasks.len() > 64;
            let items = tasks
                .into_iter()
                .take(64)
                .map(|task| Item {
                    title: format!("{} {}", if task.done { "[x]" } else { "[ ]" }, task.text),
                    counter: Some(task.line.to_string()),
                    target: Target::Note(request.document.clone(), None),
                    reasons: Vec::new(),
                })
                .collect();
            Listing {
                generation: request.generation,
                notice: if more {
                    Some(t("extensionLab.tasksTruncated"))
                } else {
                    items.is_empty().then(|| t("extensionLab.tasksEmpty"))
                },
                items,
            }
        }
        Err(_) => Listing {
            generation: request.generation,
            items: Vec::new(),
            notice: Some(t("extensionLab.tasksUnavailable")),
        },
    }
}
