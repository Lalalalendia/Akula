use masonry::core::{PaintCtx, PropertiesRef, render_text};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, RoundedRect, Vec2};
use masonry::properties::{ContentColor, SelectionColor};
use super::Fold;
use crate::ui::editor::markdown;
use super::TextEditor;
use crate::ui::theme;
use crate::ui::tokens::size;

impl TextEditor {
    pub(super) fn do_paint(&mut self, ctx: &mut PaintCtx<'_>, props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        let _zone = aq_trace::zone("редактор: отрисовка текста");
        let (top, bottom) = super::visible(ctx.window_transform(), ctx.window_size());
        let (margin, keep) = (bottom - top, self.doc.keep);
        self.painted = ((top - margin).max(keep.0), (bottom + margin).min(keep.1));
        let (first, last) = (self.doc.at_y(self.painted.0), self.doc.at_y(self.painted.1));
        if self.laid_out() {
            let (fcx, lcx) = ctx.text_contexts();
            self.doc.ensure(first..=last, fcx, lcx, &self.styles, self.focused.then_some(self.focus));
        }
        // IMG-PERF-08: prove which synthetic image paragraphs the real
        // editor considers visible, and whether an actual MediaView exists.
        // Resolver Ready alone does not imply visibility or materialization.
        #[cfg(not(feature = "installed"))]
        {
            static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
            if *ENABLED.get_or_init(|| {
                std::env::var("AKULA_EXPERIMENTAL_IMAGE_RESOLVE_PROOF")
                    .ok().as_deref() == Some("1")
            }) {
                static SEEN: std::sync::OnceLock<
                    std::sync::Mutex<std::collections::HashSet<(usize, bool, u8)>>
                > = std::sync::OnceLock::new();
                let seen = SEEN.get_or_init(Default::default);
                for i in first..=last {
                    let paragraph = self.doc.paragraph(i);
                    if !matches!(paragraph.block, markdown::Block::Image) {
                        continue;
                    }
                    let source = self.doc.slice(paragraph.range());
                    let number = source.find("photo_").and_then(|at| {
                        source[at + 6..].get(..3).and_then(|v| v.parse::<usize>().ok())
                    }).filter(|&n| n < 1000);
                    if let Some(number) = number {
                        let embedded = paragraph.embed.is_some();
                        let fold_key = match paragraph.fold {
                            Fold::None => 0u8,
                            Fold::Head => 1,
                            Fold::Member => 2,
                            Fold::Above => 3,
                        };
                        if seen.lock().unwrap_or_else(|e| e.into_inner())
                            .insert((number, embedded, fold_key))
                        {
                            eprintln!(
                                "AKULA_IMAGE_EDITOR_VISIBLE index={number} embedded={embedded} fold={:?} top={:.1} window_top={top:.1} window_bottom={bottom:.1}",
                                paragraph.fold, self.doc.top(i)
                            );
                        }
                    }
                }
            }
        }
        self.prepare_code_paint_offsets(first, last);
        let shown = first..=last;
        let texts: Vec<usize> = shown.clone().filter(|&i| {
            let p = self.doc.paragraph(i);
            p.laid().is_some() && !matches!((p.fold, &p.embed), (Fold::Member, _) | (Fold::Head, Some(_)))
        }).collect();
        let color = props.get::<ContentColor>(ctx.property_cache()).color;
        let tm = theme::current();
        let palette = markdown::palette(&tm, color);
        let markdown = self.doc.markdown.is_some();
        if markdown {
            self.paint_blocks(painter, first, last);
            for &i in &texts {
                self.paint_backdrop(painter, i);
            }
        }
        let range = self.selection();
        if !range.is_empty() && ctx.is_focus_target() {
            let sel_color = props.get::<SelectionColor>(ctx.property_cache()).color;
            self.paint_range(painter, shown.clone(), range.clone(), sel_color);
            self.paint_selection_matches(painter, shown.clone(), &range);
        }
        self.paint_extra_selections(painter, shown.clone(), props.get::<SelectionColor>(ctx.property_cache()).color);
        self.paint_hover_terms(painter, shown.clone());
        self.paint_external_reveal(painter, shown.clone());
        for found in self.find_marks(&shown) {
            self.paint_range(painter, shown.clone(), found.clone(), tm.search_match_bg);
        }
        for (i, top, _) in self.doc.placed(shown) {
            let p = self.doc.paragraph(i);
            let (Fold::Head | Fold::Above, Some(embed)) = (p.fold, &p.embed) else { continue };
            let mut overlay = match &self.table {
                Some(session) if session.start == p.start => session.overlay(self.hovered_table == Some(p.start)),
                _ => super::super::table::Overlay { hovered: self.hovered_table == Some(p.start), scroll_x: self.table_scroll_x(p.start), ..Default::default() },
            };
            overlay.search = self.block_search(p.start..self.doc.folded_end(i));
            let media = self.media_overlay(p.start);
            if media.video.is_none() {
                let hover = self.hovered_link.filter(|(start, _)| *start == p.start).map(|(_, hit)| hit);
                embed.view.paint(painter, self.shift().x, top, &palette, &overlay, &media, hover);
            }
            self.paint_link_hover(painter, i);
        }
        for i in texts {
            let origin = self.shift() + self.doc.origin(i) - Vec2::new(self.code_offset_at(i), 0.0);
            if let Some(viewport) = self.code_viewport(i) {
                // The clipped sub-painter has an independent borrow lifetime.
                // Use an inline closure, not a pretyped Painter<'_> callback.
                painter.with_fill_clip(viewport, |p| {
                    if markdown {
                        self.paint_marks(p, i, &palette);
                    }
                    render_text(p, Affine::translate(origin), self.doc.paragraph(i).layout(), &palette, true);
                });
            } else {
                if markdown {
                    self.paint_marks(painter, i, &palette);
                }
                render_text(painter, Affine::translate(origin), self.doc.paragraph(i).layout(), &palette, true);
            }
        }
        // Drop computed indices before event handling or a different document
        // window could make the same line index refer to different source.
        self.paint_code_offsets.clear();
    }

    pub(super) fn do_post_paint(&mut self, ctx: &mut PaintCtx<'_>, _props: &PropertiesRef<'_>, painter: &mut Painter<'_>) {
        if !self.videos.is_empty() && self.laid_out() {
            self.paint_videos(painter);
        }
        if !(ctx.is_focus_target() && ctx.is_window_focused() && self.laid_out()) {
            return;
        }
        if self.table_active() {
            self.paint_table_caret(painter);
            return;
        }
        if !self.caret_visible {
            return;
        }
        let caret = self.caret();
        let ink = theme::current().caret_color;
        if let Some(viewport) = self.code_viewport(self.doc.at_offset(self.focus)) {
            painter.with_fill_clip(viewport, |p| {
                p.fill(RoundedRect::from_rect(caret, size::CARET_RADIUS), ink).draw();
            });
        } else {
            painter.fill(RoundedRect::from_rect(caret, size::CARET_RADIUS), ink).draw();
        }
        self.paint_extra_carets(painter, ink);
    }

}
