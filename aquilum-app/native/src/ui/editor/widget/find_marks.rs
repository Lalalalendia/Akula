use super::*;

impl TextEditor {
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

}
