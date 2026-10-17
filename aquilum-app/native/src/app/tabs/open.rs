use std::path::{Path, PathBuf};

use aquilum_core::search::paths::{is_markdown, is_plain_text};
use masonry::app::RenderRoot;
use masonry::widgets::IndexedStack;

use super::super::{App, Outcome};
use crate::note::Note;
use crate::ui::editor::TextEditor;
use crate::ui::widgets::Slot;

// Test-only instrumentation. The production UI path and API stay unchanged.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TabSwitchStages {
    pub leave: std::time::Duration,
    pub file_read: std::time::Duration,
    pub page_build: std::time::Duration,
    pub tree: std::time::Duration,
    pub tab: std::time::Duration,
    pub version: std::time::Duration,
    pub strip: std::time::Duration,
    pub panel: std::time::Duration,
}

#[cfg(test)]
thread_local! {
    static LAST_TAB_SWITCH: std::cell::Cell<TabSwitchStages> =
        std::cell::Cell::new(TabSwitchStages::default());
}

#[cfg(test)]
pub(crate) fn take_tab_switch_stages() -> TabSwitchStages {
    LAST_TAB_SWITCH.with(|slot| slot.replace(TabSwitchStages::default()))
}

#[cfg(test)]
fn record_stage(update: impl FnOnce(&mut TabSwitchStages)) {
    LAST_TAB_SWITCH.with(|slot| {
        let mut current = slot.get();
        update(&mut current);
        slot.set(current);
    });
}

impl App {
    pub(in crate::app) fn show_active(&mut self, root: &mut RenderRoot) {
        #[cfg(test)]
        LAST_TAB_SWITCH.with(|slot| slot.set(TabSwitchStages::default()));
        if let Some((layer, _)) = self.suggest_layer.take() {
            root.remove_layer(layer);
        }
        #[cfg(test)]
        let started = std::time::Instant::now();
        self.show_tab(root);
        #[cfg(test)]
        record_stage(|s| s.tab = started.elapsed());

        #[cfg(test)]
        let started = std::time::Instant::now();
        self.version_view = None;
        self.show_version(root);
        #[cfg(test)]
        record_stage(|s| s.version = started.elapsed());

        #[cfg(test)]
        let started = std::time::Instant::now();
        self.refresh_tabs(root);
        #[cfg(test)]
        record_stage(|s| s.strip = started.elapsed());

        #[cfg(test)]
        let started = std::time::Instant::now();
        self.request_panel(root);
        #[cfg(test)]
        record_stage(|s| s.panel = started.elapsed());

        self.workspace.core.active_note.set(self.note.as_ref().map(|n| n.path.clone()));
    }

    fn show_tab(&mut self, root: &mut RenderRoot) {
        let tab = self.tabs.active().clone();
        if tab.graph && self.tree.root().is_some() {
            self.leave_note();
            self.drop_note_page(root);
            self.tree.set_active(root, None);
            self.show_graph(root);
            self.show_page(root, 4);
            return;
        }
        let Some(path) = tab.path else {
            self.leave_note();
            self.drop_note_page(root);
            self.tree.set_active(root, None);
            self.show_page(root, if self.tree.root().is_some() { 1 } else { 3 });
            return;
        };
        if self.note.as_ref().is_some_and(|n| n.is(&path)) {
            self.reveal_pending(root);
            self.show_page(root, 0);
            return;
        }
        #[cfg(test)]
        let started = std::time::Instant::now();
        self.leave_note();
        #[cfg(test)]
        record_stage(|s| s.leave = started.elapsed());

        let read = aq_trace::zone("открытие: чтение файла");
        #[cfg(test)]
        let started = std::time::Instant::now();
        let opened = Note::open(path.clone());
        #[cfg(test)]
        record_stage(|s| s.file_read = started.elapsed());
        drop(read);
        match opened {
            Ok((note, text)) => {
                self.open_error = None;
                self.open_error_ids = None;
                self.text = text;
                self.note = Some(note);
                let _zone = aq_trace::zone("открытие: страница заметки");
                #[cfg(test)]
                let started = std::time::Instant::now();
                self.render_note(root);
                self.reveal_pending(root);
                #[cfg(test)]
                record_stage(|s| s.page_build = started.elapsed());
            }
            Err(error) => {
                let err_str = format!("{error}");
                report!("заметка {} не прочитана: {err_str}", path.display());
                self.open_error = Some((path.clone(), err_str.clone()));
                self.show_open_error(root, &path, &err_str);
            }
        }
        self.hold = Some(std::time::Instant::now() + super::super::HOLD);
        #[cfg(test)]
        let started = std::time::Instant::now();
        self.tree.set_active(root, Some(path));
        self.show_page(root, 0);
        #[cfg(test)]
        record_stage(|s| s.tree = started.elapsed());
    }

    pub(in crate::app) fn show_open_error(&mut self, root: &mut RenderRoot, path: &Path, error: &str) {
        self.note = None;
        self.note_ids = None;
        self.embeds = None;
        let (page, ids) = crate::ui::note::open_error_page(path, error);
        self.open_error_ids = Some(ids);
        if let Some(chrome) = &self.chrome {
            chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
        }
    }

    fn drop_note_page(&mut self, root: &mut RenderRoot) {
        self.embeds = None;
        if self.note_ids.take().is_none() {
            return;
        }
        self.text = crop::Rope::new();
        if let Some(chrome) = &self.chrome {
            chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, masonry::core::NewWidget::new(masonry::widgets::SizedBox::empty()).erased()));
        }
    }

    pub(in crate::app) fn leave_note(&mut self) {
        self.flush_view();
        self.view = None;
        self.restore = None;
        if let Some(note) = self.note.take() {
            note.leave(&self.workspace.core, &self.text);
        }
    }

    pub(in crate::app) fn show_page(&self, root: &mut RenderRoot, index: usize) {
        if let Some(chrome) = &self.chrome {
            chrome.pages.edit(root, |mut pages| {
                if pages.widget.active_child() != index {
                    IndexedStack::set_active_child(&mut pages, index);
                }
            });
        }
    }

    pub(crate) fn open(&mut self, root: &mut RenderRoot, path: PathBuf) -> bool {
        self.open_in(root, path, false)
    }

    fn reveal_pending(&mut self, root: &mut RenderRoot) {
        let Some(units) = self.pending_reveal.take() else { return };
        let byte = self.byte_of(usize::try_from(units).unwrap_or(0)).saturating_sub(self.hidden);
        self.reveal(root, byte);
        if let Some(ids) = &self.note_ids {
            ids.body.edit(root, |mut b| TextEditor::select_byte_range(&mut b, byte, byte));
        }
    }

    pub(in crate::app) fn open_in(&mut self, root: &mut RenderRoot, path: PathBuf, new_tab: bool) -> bool {
        self.open_in_with_offset(root, path, new_tab, None)
    }

    pub(crate) fn open_in_with_offset(&mut self, root: &mut RenderRoot, path: PathBuf, new_tab: bool, offset: Option<usize>) -> bool {
        if !is_markdown(&path) && !is_plain_text(&path) {
            return false;
        }
        if let Some(current) = self.note.as_ref().map(|n| n.path.clone()) {
            self.navigation.push(&current);
        }
        self.navigation.push(&path);
        self.pending_reveal = offset;
        if new_tab {
            self.tabs.open_new(path);
        } else {
            self.tabs.open(path);
        }
        self.show_active(root);
        true
    }

    pub(in crate::app) fn new_tab(&mut self, root: &mut RenderRoot) -> Outcome {
        self.tabs.new_tab();
        self.show_active(root);
        Outcome { title: true, ..Outcome::default() }
    }

    pub(in crate::app) fn close_active_tab(&mut self, root: &mut RenderRoot) -> Outcome {
        self.tabs.close(self.tabs.active_id());
        self.show_active(root);
        Outcome { title: true, ..Outcome::default() }
    }
}
