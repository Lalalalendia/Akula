mod readonly;
mod open_in;
mod docx;
#[cfg(feature = "dicom-preview")]
mod dicom;
use std::path::{Path, PathBuf};
use aquilum_core::search::paths::{is_markdown, is_plain_text};
use masonry::app::RenderRoot;
use masonry::widgets::IndexedStack;
use super::super::{App, Outcome};
use crate::note::Note;
use crate::ui::editor::TextEditor;
use crate::ui::widgets::Slot;
fn is_structured_file(path: &Path) -> bool {
    path.extension().and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("canvas") || ext.eq_ignore_ascii_case("base"))
}
const MAX_INACTIVE_NOTES: usize = 2;
const MAX_INACTIVE_BYTES: usize = 8 * 1024 * 1024;
/// Clean text only. A cached Rope is *never* authoritative without comparing
/// its raw-file BLAKE3 digest against the actual bytes on every reopening.
pub(in crate::app) struct CachedNote {
    tab_id: uuid::Uuid,
    path: PathBuf,
    hash: String,
    text: crop::Rope,
    #[cfg(test)]
    layout: Option<crate::ui::editor::LayoutTrialSnapshot>,
}

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
    pub cache_hit: bool,
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
        #[cfg(feature = "dicom-preview")]
        dicom::retain_dicom_grants(&mut self.dicom_external_sources, &self.tabs);
        // Closing/renaming a tab also invalidates any cached text associated
        // with its old identity; workspace switches clear the cache outright.
        self.inactive_notes.retain(|entry| {
            self.tabs.tabs().iter().any(|tab| tab.id == entry.tab_id && tab.path.as_deref() == Some(entry.path.as_path()))
        });
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
        self.discard_version_view(root);
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
        if crate::images::kind(&path.to_string_lossy()) == Some(crate::images::MediaKind::Image) {
            self.leave_note();
            self.drop_note_page(root);
            self.show_image_readonly(root, &path);
            self.tree.set_active(root, Some(path));
            self.show_page(root, 0);
            return;
        }
        #[cfg(feature = "dicom-preview")]
        if crate::dicom::is_candidate(&path) || self.dicom_external_sources.contains(&path) { return self.show_dicom_tab(root, path); }
        if crate::docx::is_docx_path(&path) { return self.show_docx_tab(root, path); }
        if is_structured_file(&path) {
            self.leave_note();
            self.show_structured_readonly(root, &path);
            self.tree.set_active(root, Some(path));
            self.show_page(root, 0);
            return;
        }
        if self.note.as_ref().is_some_and(|n| n.is(&path)) {
            self.reveal_pending(root);
            self.show_page(root, 0);
            return;
        }
        #[cfg(test)]
        let started = std::time::Instant::now();
        #[cfg(test)]
        {
            // Reading WidgetRef does not trigger framework edit/rewrite passes.
            // Test-only: never intercept a production tab switch.
            self.parley_capture = if self.parley_trial_enabled {
                self.note_ids
                    .as_ref()
                    .and_then(|ids| ids.body.get(root))
                    .and_then(|widget| widget.parley_trial_snapshot())
            } else {
                None
            };
            if self.parley_trial_enabled {
                eprintln!("PARLEY_TAB_TRACE stage=capture found={}", self.parley_capture.is_some());
            }
        }
        self.leave_note();
        #[cfg(test)]
        record_stage(|s| s.leave = started.elapsed());
        let read = aq_trace::zone("открытие: чтение файла");
        #[cfg(test)]
        let started = std::time::Instant::now();
        let opened = self.open_cached_or_read(tab.id, path.clone());
        #[cfg(test)]
        record_stage(|s| s.file_read = started.elapsed());
        drop(read);
        match opened {
            Ok((mut note, text)) => {
                self.arm_note_recovery(&mut note);
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
        self.embeds = None; self.open_error = Some((path.to_path_buf(), error.to_owned()));
        let (page, ids) = crate::ui::note::open_error_page(path, error);
        self.open_error_ids = Some(ids);
        if let Some(chrome) = &self.chrome {
            chrome.note_page.edit(root, |mut slot| Slot::set_child(&mut slot, page.erased()));
        }
    }

    fn drop_note_page(&mut self, root: &mut RenderRoot) {
        self.embeds = None;
        // Read-only image/structured pages have no NoteIds, but their decoded
        // content must still be released when the tab is closed or switched.
        self.note_ids = None;
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
            let snapshot = note.clean_cache_hash().and_then(|hash| {
                self.tabs.tabs().iter()
                    .find(|tab| tab.path.as_deref() == Some(note.path.as_path()))
                    .map(|tab| (tab.id, hash))
            });
            if let Some((tab_id, hash)) = snapshot {
                self.remember_clean_note(tab_id, note.path.clone(), hash, self.text.clone());
            }
            note.leave(&self.workspace.core, &self.text);
        }
    }

    fn remember_clean_note(&mut self, tab_id: uuid::Uuid, path: PathBuf, hash: String, text: crop::Rope) {
        self.inactive_notes.retain(|entry| entry.tab_id != tab_id);
        if text.byte_len() > MAX_INACTIVE_BYTES {
            return;
        }
        self.inactive_notes.push(CachedNote {
            tab_id,
            path,
            hash,
            text,
            #[cfg(test)]
            layout: if self.parley_trial_enabled { self.parley_capture.take() } else { None },
        });
        while self.inactive_notes.len() > MAX_INACTIVE_NOTES
            || self.inactive_notes.iter().map(|entry| entry.text.byte_len()).sum::<usize>() > MAX_INACTIVE_BYTES
        {
            self.inactive_notes.remove(0);
        }
    }

    fn open_cached_or_read(&mut self, tab_id: uuid::Uuid, path: PathBuf) -> Result<(Note, crop::Rope), aquilum_core::files::error::FileCommandError> {
        #[cfg(test)]
        {
            self.parley_offer = None;
        }
        if let Some(index) = self.inactive_notes.iter().position(|entry| entry.tab_id == tab_id && entry.path == path) {
            let cached = self.inactive_notes.remove(index);
            // Size and mtime are not a correctness boundary: external writers
            // may preserve both. Validate every byte via streaming BLAKE3.
            let matches_disk = aquilum_core::files::document::read_file_hash_impl(&path)
                .is_ok_and(|digest| digest == cached.hash);
            if matches_disk {
                #[cfg(test)]
                {
                    record_stage(|stage| stage.cache_hit = true);
                    if self.parley_trial_enabled {
                        eprintln!("PARLEY_TAB_TRACE stage=verified_cache candidate={}", cached.layout.is_some());
                        // The original Rope hash has just been checked against
                        // disk; an unverified or edited file never receives
                        // the cached Parley layouts.
                        self.parley_offer = cached.layout;
                    }
                }
                return Ok(Note::from_verified_cache(path, cached.text, cached.hash));
            }
        }
        Note::open(path)
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

    pub(in crate::app) fn new_tab(&mut self, root: &mut RenderRoot) -> Outcome {
        self.tabs.new_tab();
        self.show_active(root);
        Outcome { title: true, ..Outcome::default() }
    }

    pub(in crate::app) fn close_active_tab(&mut self, root: &mut RenderRoot) -> Outcome {
        if self.canvas_dirty_active() { self.canvas_notify(root, "Canvas has unsaved changes. Save it before closing."); return Outcome::default(); }
        self.tabs.close(self.tabs.active_id());
        self.show_active(root);
        Outcome { title: true, ..Outcome::default() }
    }
}

#[cfg(test)]
mod structured_tab_tests {
    use super::is_structured_file;
    use std::path::Path;

    #[test]
    fn structured_files_are_routed_separately_from_markdown() {
        assert!(is_structured_file(Path::new("board.canvas")));
        assert!(is_structured_file(Path::new("board.CANVAS")));
        assert!(is_structured_file(Path::new("items.base")));
        assert!(is_structured_file(Path::new("items.BASE")));
        assert!(!is_structured_file(Path::new("note.md")));
        assert!(!is_structured_file(Path::new("report.pdf")));
    }
}
