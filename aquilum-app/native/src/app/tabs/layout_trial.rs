//! Test-only UI integration of the bounded plain-Parley snapshot experiment.
//! Never constructed or called in normal release binaries.
use std::path::{Path, PathBuf};

use masonry::app::RenderRoot;
use uuid::Uuid;

use super::super::App;
use crate::ui::editor::{TextEditor, TrialSnapshot};

const MAX_LAYOUT_CANDIDATES: usize = 2;

// Test-only observability to distinguish an ineligible document from a
// rejected cache context. No diagnostics are built into release binaries.
#[derive(Clone, Copy, Default)]
pub(crate) struct TrialCounts {
    pub(crate) attempted: usize,
    pub(crate) clean: usize,
    pub(crate) widget_found: usize,
    pub(crate) snapshots: usize,
    pub(crate) offered: usize,
}

thread_local! {
    static COUNTS: std::cell::Cell<TrialCounts> = const { std::cell::Cell::new(TrialCounts {
        attempted: 0, clean: 0, widget_found: 0, snapshots: 0, offered: 0
    }) };
}

fn count(update: impl FnOnce(&mut TrialCounts)) {
    COUNTS.with(|slot| {
        let mut counts = slot.get();
        update(&mut counts);
        slot.set(counts);
    });
}

pub(crate) fn take_counts() -> TrialCounts {
    COUNTS.with(|slot| slot.replace(TrialCounts::default()))
}

pub(in crate::app) struct LayoutCandidate {
    tab_id: Uuid,
    path: PathBuf,
    snapshot: TrialSnapshot,
}

pub(in crate::app) fn enabled() -> bool {
    matches!(std::env::var("AQ_TEST_PARLEY_UI_REUSE").as_deref(), Ok("1"))
}

impl App {
    /// Capture before the old subtree is removed. Only clean documents are
    /// eligible and any snapshot is only a candidate until BLAKE3 revalidation.
    pub(in crate::app) fn trial_capture_inactive(&mut self, root: &RenderRoot) {
        if !enabled() {
            return;
        }
        count(|c| c.attempted += 1);
        let Some(note) = &self.note else { return };
        if note.clean_cache_hash().is_none() {
            return;
        }
        count(|c| c.clean += 1);
        let Some(tab_id) = self.tabs.tabs().iter()
            .find(|tab| tab.path.as_deref() == Some(note.path.as_path()))
            .map(|tab| tab.id)
        else { return };
        let Some(ids) = &self.note_ids else { return };
        let Some(widget) = root.get_widget(ids.body.id()).and_then(|w| w.downcast::<TextEditor>()) else { return };
        count(|c| c.widget_found += 1);
        let Some(snapshot) = widget.inner().trial_plain_layout_snapshot() else { return };
        count(|c| c.snapshots += 1);
        self.inactive_plain_layouts.retain(|entry| entry.tab_id != tab_id);
        self.inactive_plain_layouts.push(LayoutCandidate { tab_id, path: note.path.clone(), snapshot });
        if self.inactive_plain_layouts.len() > MAX_LAYOUT_CANDIDATES {
            self.inactive_plain_layouts.remove(0);
        }
    }

    /// The caller must first pass the ordinary raw-byte BLAKE3 disk freshness
    /// gate. Size+mtime or a surviving WidgetId are never sufficient.
    pub(in crate::app) fn trial_take_verified(&mut self, tab_id: Uuid, path: &Path) -> Option<TrialSnapshot> {
        if !enabled() {
            return None;
        }
        let index = self.inactive_plain_layouts.iter()
            .position(|candidate| candidate.tab_id == tab_id && candidate.path == path)?;
        count(|c| c.offered += 1);
        Some(self.inactive_plain_layouts.remove(index).snapshot)
    }

    pub(in crate::app) fn trial_prune_closed(&mut self) {
        if !enabled() {
            self.inactive_plain_layouts.clear();
            self.pending_plain_layout = None;
            return;
        }
        self.inactive_plain_layouts.retain(|entry| self.tabs.tabs().iter()
            .any(|tab| tab.id == entry.tab_id && tab.path.as_deref() == Some(entry.path.as_path())));
    }
}
