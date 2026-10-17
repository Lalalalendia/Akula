use super::*;

impl TextEditor {
    pub fn selected(&self) -> String {
        self.selected_text().unwrap_or_default()
    }

    pub fn image_source_shown(&self) -> bool {
        self.doc.source.is_some()
    }

    pub fn inline_boxes(&self, line: usize) -> usize {
        self.doc.paragraph(line).decor().boxes.len()
    }

    pub fn brush_at(&self, offset: usize) -> Option<BrushIndex> {
        let (_, local, layout) = self.local(offset);
        layout?.lines().flat_map(|line| line.items().collect::<Vec<_>>()).find_map(|item| match item {
            masonry::parley::PositionedLayoutItem::GlyphRun(run) if run.run().text_range().contains(&local) => Some(run.style().brush.clone()),
            _ => None,
        })
    }
}

impl TextEditor {
    pub(crate) fn parley_trial_snapshot(&self) -> Option<super::super::LayoutTrialSnapshot> {
        self.doc.trial_snapshot(&self.styles)
    }

    pub(crate) fn with_parley_trial(mut self, snapshot: super::super::LayoutTrialSnapshot) -> Self {
        self.doc.trial_offer(snapshot);
        self
    }
}
