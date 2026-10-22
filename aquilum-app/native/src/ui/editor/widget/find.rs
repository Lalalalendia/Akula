use std::sync::mpsc::{Receiver, channel};

use aquilum_core::text::cancel::Generation;
use aquilum_core::text::matcher::Matcher;

use super::matching::{FIND, find_in, find_terms};
use super::*;
use crate::ui::editor::change::{self, Change};

pub type Status = (usize, usize, bool);

#[derive(Default)]
pub(super) struct Find {
    query: String,
    matches: Vec<Range<usize>>,
    active: usize,
    only_active: bool,
    generation: Generation,
    pending: Option<Receiver<Vec<Range<usize>>>>,
}

impl TextEditor {
    fn find_status(&self) -> Status {
        let total = self.find.matches.len();
        (if total == 0 { 0 } else { self.find.active + 1 }, total, total == FIND.limit)
    }

    /// Cheap search and layout signature for the scrollbar markers.
    pub fn find_mark_stamp(&self) -> Option<(u64, usize, bool, (usize, usize), f64, f64)> {
        if self.find.query.is_empty() {
            return None;
        }
        Some((
            self.find.generation.current(),
            self.find.matches.len(),
            self.find.pending.is_some(),
            self.doc.loaded_range(),
            self.width,
            self.doc.height(),
        ))
    }

    /// Y coordinates within the text widget (not the whole scrollable page).
    pub fn find_mark_positions(&self) -> Vec<f64> {
        if self.find.pending.is_some() {
            return Vec::new();
        }
        let (loaded_start, loaded_end) = self.doc.loaded_range();
        self.find.matches.iter().map(|range| {
            let byte = range.start;
            if byte < loaded_start || byte >= loaded_end {
                return self.doc.y_of(byte);
            }
            let line = self.doc.at_offset(byte);
            let paragraph = self.doc.paragraph(line);
            if let Some(layout) = paragraph.laid() {
                let local = (byte - paragraph.start).min(paragraph.len as usize);
                let caret = Cursor::from_byte_index(layout, local, Affinity::Downstream).geometry(layout, 0.0);
                let y = self.doc.origin(line).y + caret.y0;
                if y.is_finite() {
                    return y;
                }
            }
            let top = self.doc.top(line);
            let fraction = (byte - paragraph.start) as f64 / f64::from(paragraph.len.max(1));
            top + (self.doc.bottom(line) - top).max(0.0) * fraction
        }).collect()
    }

    fn search(&mut self) {
        let cancel = self.find.generation.next();
        self.find.pending = None;
        let matcher = Matcher::any(&find_terms(&self.find.query), FIND);
        let (base, end) = self.doc.loaded_range();
        if matcher.is_empty() || base == 0 && end == self.doc.len() {
            self.find.matches = self.doc.search(|text| matcher.find_all(text));
        } else {
            let (tx, rx) = channel();
            let (rope, wake) = (self.doc.rope(), self.wake.clone());
            std::thread::spawn(move || {
                let _zone = aq_trace::zone("фон: поиск по заметке");
                if tx.send(find_in(&rope, &matcher, &cancel)).is_ok()
                    && let Some(wake) = wake
                {
                    wake();
                }
            });
            self.find.pending = Some(rx);
        }
        self.find.active = self.find.active.min(self.find.matches.len().saturating_sub(1));
    }

    pub(super) fn refind(&mut self, changes: &[Change]) {
        if self.find.query.is_empty() {
            return;
        }
        for found in &mut self.find.matches {
            *found = change::map(found.start, changes)..change::map(found.end, changes);
        }
        self.search();
    }

    pub(super) fn block_search(&self, block: Range<usize>) -> Option<Search> {
        if self.find.query.is_empty() {
            return None;
        }
        let query = self.find.query.clone();
        if !self.find.only_active {
            return Some(Search { query, only: None });
        }
        let active = self.find.matches.get(self.find.active)?;
        let before = self.find.matches.partition_point(|m| m.start < block.start);
        (block.start <= active.start && active.end <= block.end).then(|| Search { query, only: Some(self.find.active - before) })
    }

    pub(super) fn find_marks(&self, shown: &std::ops::RangeInclusive<usize>) -> &[Range<usize>] {
        let matches = &self.find.matches;
        match matches.get(self.find.active) {
            Some(active) if self.find.only_active => std::slice::from_ref(active),
            _ => {
                let visible = self.shown_bytes(shown);
                &matches[matches.partition_point(|m| m.end < visible.start)..matches.partition_point(|m| m.start <= visible.end)]
            }
        }
    }

    pub fn find(this: &mut WidgetMut<'_, Self>, query: &str) -> Status {
        let find = &mut this.widget.find;
        (find.query, find.active, find.only_active) = (query.to_owned(), 0, false);
        find.matches.clear();
        this.widget.search();
        this.ctx.request_render();
        this.widget.find_status()
    }

    pub fn poll_find(this: &mut WidgetMut<'_, Self>) -> Option<Status> {
        let found = this.widget.find.pending.as_ref()?.try_recv().ok()?;
        let find = &mut this.widget.find;
        find.pending = None;
        find.active = find.active.min(found.len().saturating_sub(1));
        find.matches = found;
        this.ctx.request_render();
        Some(this.widget.find_status())
    }

    pub fn find_step(this: &mut WidgetMut<'_, Self>, delta: isize) -> (Status, Option<usize>) {
        let total = this.widget.find.matches.len();
        if total > 0 {
            let find = &mut this.widget.find;
            find.active = crate::ui::keymap::cycle(find.active, total, delta > 0);
            find.only_active = false; // Keep every match visible after next/previous.
            this.ctx.request_render();
        }
        let start = this.widget.find.matches.get(this.widget.find.active).map(|m| m.start);
        (this.widget.find_status(), start)
    }

    pub fn find_show_all(this: &mut WidgetMut<'_, Self>) {
        this.widget.find.only_active = false;
        this.ctx.request_render();
    }
}
