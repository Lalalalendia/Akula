use masonry::app::RenderRoot;
use super::App;
impl App {
    pub(super) fn refresh_find_marks(&mut self, root: &mut RenderRoot) {
        if self.find.is_none() && self.find_mark_stamp.is_none() {
            return;
        }
        let Some(ids) = &self.note_ids else {
            self.find_mark_stamp = None;
            return;
        };
        let scroll = ids.scroll;
        let body = ids.body;
        if self.find.is_none() {
            if self.find_mark_stamp.take().is_some() {
                scroll.edit(root, |mut area| crate::ui::scroll::ScrollArea::set_find_marks(&mut area, Vec::new()));
            }
            return;
        }
        let Some((viewport_y, editor_y, offset)) = self.scroll_geometry(root) else { return };
        let Some(source) = body.get(root).and_then(|b| b.inner().find_mark_stamp()) else {
            if self.find_mark_stamp.take().is_some() {
                scroll.edit(root, |mut area| crate::ui::scroll::ScrollArea::set_find_marks(&mut area, Vec::new()));
            }
            return;
        };
        let Some(height) = scroll.get(root).map(|area| area.inner().content_height()) else { return };
        let origin = editor_y - viewport_y + offset;
        let stamp = super::find_marks::MarkStamp { source, body_origin: origin, scroll_height: height };
        if self.find_mark_stamp.is_some_and(|old| {
            old.source == stamp.source
                && old.scroll_height == stamp.scroll_height
                && (old.body_origin - stamp.body_origin).abs() < 1.0
        }) {
            return;
        }
        let positions = body.get(root).map(|b| b.inner().find_mark_positions()).unwrap_or_default();
        let marks = positions.into_iter().filter_map(|y| {
            let fraction = (origin + y) / height;
            fraction.is_finite().then_some(fraction.clamp(0.0, 1.0))
        }).collect();
        scroll.edit(root, |mut area| crate::ui::scroll::ScrollArea::set_find_marks(&mut area, marks));
        self.find_mark_stamp = Some(stamp);
    }

}
