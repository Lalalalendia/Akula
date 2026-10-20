use std::time::{Duration, Instant};

use masonry::app::RenderRoot;

use super::super::App;

pub const VIEW_SAVE_DELAY: Duration = Duration::from_millis(500);

impl App {
    pub(in crate::app) fn scroll_geometry(&self, root: &RenderRoot) -> Option<(f64, f64, f64)> {
        let ids = self.note_ids.as_ref()?;
        let scroll = root.get_widget(ids.scroll.id())?;
        let body = root.get_widget(ids.body.id())?;
        let top = |t: masonry::kurbo::Affine| (t * masonry::kurbo::Point::ZERO).y;
        let offset = scroll.downcast::<crate::ui::scroll::ScrollArea<masonry::widgets::Flex>>()?.inner().offset();
        Some((top(scroll.ctx().window_transform()), top(body.ctx().window_transform()), offset))
    }

    pub(in crate::app) fn capture_view(&mut self, root: &RenderRoot) {
        let Some((viewport, editor, _)) = self.scroll_geometry(root) else { return };
        let Some(ids) = &self.note_ids else { return };
        let Some(body) = root.get_widget(ids.body.id()).and_then(|w| w.downcast::<crate::ui::editor::TextEditor>()) else { return };
        let Some((offset, px)) = body.inner().anchor_at(viewport - editor) else { return };
        self.view = Some((self.hidden + offset, px));
        self.view_due = Some(Instant::now() + VIEW_SAVE_DELAY);
    }

    pub fn restore_view(&mut self, root: &mut RenderRoot) -> bool {
        let Some((anchor, px)) = self.restore else { return false };
        let Some((viewport, editor, offset)) = self.scroll_geometry(root) else { return false };
        let Some(ids) = &self.note_ids else { return false };
        let byte = anchor.saturating_sub(self.hidden);
        let Some(top) = root.get_widget(ids.body.id()).and_then(|w| w.downcast::<crate::ui::editor::TextEditor>()).and_then(|b| b.inner().top_of(byte)) else {
            return false;
        };
        let target = editor - viewport + offset + top + px;
        let scroll = ids.scroll;
        scroll.edit(root, |mut area| crate::ui::scroll::ScrollArea::set_offset(&mut area, target));
        self.restore = None;
        true
    }

    /// Visible-note-only developer stress driver. The actual scroll target
    /// is resolved by the editor's own byte-to-layout geometry, not by a
    /// guessed wheel offset. Return None unless the anchor can be laid out.
    #[cfg(not(feature = "installed"))]
    pub(crate) fn image_scan_reveal(&mut self, root: &mut RenderRoot, byte: usize) -> Option<(f64, f64)> {
        // A single programmatic reveal is NOT sufficient for a document
        // whose images decode asynchronously: their real 138px layout
        // heights replace estimates after the initial anchor seek. The
        // first Win32 proof painted 10 images but 8 lay *outside* the
        // current viewport. Re-measure the target on every dwell tick
        // and only re-center when geometry actually drifts.
        let (viewport_y, editor_y, before) = self.scroll_geometry(root)?;
        let ids = self.note_ids.as_ref()?;
        let scroll = ids.scroll;
        let body = ids.body;
        let top = root.get_widget(body.id())
            .and_then(|w| w.downcast::<crate::ui::editor::TextEditor>())?
            .inner().top_of(byte)?;
        let height = root.get_widget(scroll.id())?.ctx().bounding_box().height();
        if !top.is_finite() || !height.is_finite() || height <= 0.0 {
            return None;
        }
        let error = editor_y + top - viewport_y - height * 0.5;
        if error.abs() <= 8.0 {
            // The actual paragraph position is already centered. Don't
            // request an extra layout or rebuild the retained scene just
            // to generate artificial CPU work. Record affirmative per-byte
            // alignment only after reading the latest native layout.
            static ALIGNED: std::sync::OnceLock<
                std::sync::Mutex<std::collections::HashSet<usize>>
            > = std::sync::OnceLock::new();
            if ALIGNED.get_or_init(Default::default)
                .lock().unwrap_or_else(|e| e.into_inner())
                .insert(byte)
            {
                eprintln!(
                    "AKULA_IMAGE_ANCHOR_ALIGNED byte={byte} center_error={error:.1} offset={before:.1}"
                );
            }
            return Some((before, before));
        }
        self.reveal(root, byte);
        // The normal real-wheel ScrollArea path explicitly requests Compose
        // and PostPaint when its offset changes; set_offset() used by
        // programmatic reveal requests only Layout. Mirror native wheel
        // invalidation for this developer stress driver, without modifying
        // any installed or ordinary navigation behavior.
        scroll.edit(root, |mut area| {
            area.ctx.request_compose();
            area.ctx.request_post_paint();
        });
        // A forced BGRA redraw in the native host does not necessarily
        // rebuild retained Masonry paint commands when only the parent
        // scroll translation changes. For this *developer-only* LOD stress
        // driver, invalidate the editor paint record at every new image
        // anchor. Otherwise all 10 images reach MediaView::paint during
        // some scene builds yet only the two initially raster-visible
        // brushes are requested by the Vello mip sampler.
        body.edit(root, |mut editor| editor.ctx.request_render());
        static FIRST_REPAINT: std::sync::Once = std::sync::Once::new();
        FIRST_REPAINT.call_once(|| eprintln!("AKULA_IMAGE_SCAN_EDITOR_REPAINT_REQUESTED"));
        let after = self.scroll_geometry(root)?.2;
        eprintln!(
            "AKULA_IMAGE_ANCHOR_RECENTER byte={byte} pre_error={error:.1} from={before:.1} to={after:.1}"
        );
        Some((before, after))
    }

    pub(in crate::app) fn flush_view(&mut self) {
        self.view_due = None;
        let units = self.view.map(|(anchor, px)| (self.units_of(anchor), px));
        let (Some(session), Some(note), Some((units, px))) = (self.session.as_mut(), self.note.as_ref(), units) else { return };
        if let Some(document) = session.document(&self.workspace.core, &note.path) {
            let _zone = aq_trace::zone("место в заметке: запись");
            session.save_view(&self.workspace.core, document, units as i64, px);
        }
    }

    /// Прокручивает заметку так, чтобы место `byte` текста (без метаданных) оказалось посередине.
    pub(crate) fn reveal(&mut self, root: &mut RenderRoot, byte: usize) {
        let Some((viewport, editor, offset)) = self.scroll_geometry(root) else { return };
        let Some(ids) = &self.note_ids else { return };
        let Some(top) = root.get_widget(ids.body.id()).and_then(|w| w.downcast::<crate::ui::editor::TextEditor>()).and_then(|b| b.inner().top_of(byte)) else { return };
        let height = root.get_widget(ids.scroll.id()).map_or(0.0, |w| w.ctx().bounding_box().height());
        let target = editor - viewport + offset + top - height / 2.0;
        let scroll = ids.scroll;
        scroll.edit(root, |mut area| crate::ui::scroll::ScrollArea::set_offset(&mut area, target));
    }
}
