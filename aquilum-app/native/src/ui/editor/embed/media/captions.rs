use super::{MediaOverlay, MediaView};
use masonry::core::{BrushIndex, PaintCtx, render_text};
use masonry::imaging::Painter;
use masonry::kurbo::{Affine, Rect, RoundedRect, Vec2};
use masonry::parley::style::StyleProperty;
use masonry::parley::{Alignment, AlignmentOptions, FontWeight, Layout};
use masonry::peniko::Color;
use std::{cell::RefCell, sync::Arc};
struct Cached {
    text: Arc<str>,
    width: f32,
    size: f32,
    family: String,
    layout: Layout<BrushIndex>,
}
thread_local! {
    static LAYOUT: RefCell<Option<Cached>> = const { RefCell::new(None) };
}

impl MediaView {
    pub(crate) fn paint_captions(
        &self,
        ctx: &mut PaintCtx<'_>,
        painter: &mut Painter<'_>,
        x: f64,
        y: f64,
        overlay: &MediaOverlay,
    ) {
        if overlay.crop.is_some() {
            return;
        }
        let Some(state) = overlay.video.as_ref() else {
            return;
        };
        let Some(text) = self
            .path
            .as_deref()
            .and_then(|path| crate::video::captions::text_at(path, state.position))
        else {
            return;
        };
        let frame = self.frame + Vec2::new(x, y);
        let bottom = (if overlay.hovered || !state.playing {
            self.controls().y0 + y
        } else {
            frame.y1
        }) - 8.0;
        paint_caption(ctx, painter, frame, bottom, text);
    }
}

pub(crate) fn paint_caption(ctx: &mut PaintCtx<'_>, painter: &mut Painter<'_>, frame: Rect, bottom: f64, text: Arc<str>) {
    if frame.width() < 80.0 || frame.height() < 64.0 {
        return;
    }
    let width = (frame.width() * 0.86).min(760.0).floor() as f32;
    let font = crate::ui::theme::ui_font_settings();
    let size = font
        .size
        .clamp(12.0, (frame.height() as f32 / 8.0).clamp(12.0, 24.0));
    LAYOUT.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|old| {
            old.text != text
                || old.width != width
                || old.size != size
                || old.family != font.family
        }) {
            let (fonts, layouts) = ctx.text_contexts();
            let mut builder = layouts.ranged_builder(fonts, &text, 1.0, true);
            builder.push_default(StyleProperty::FontFamily(crate::ui::theme::ui_font(
                &font.family,
            )));
            builder.push_default(StyleProperty::FontSize(size));
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(550.0)));
            let mut layout = builder.build(&text);
            layout.break_all_lines(Some(width));
            layout.align(Alignment::Center, AlignmentOptions::default());
            *cache = Some(Cached {
                text,
                width,
                size,
                family: font.family.clone(),
                layout,
            });
        }
        let layout = &cache.as_ref().expect("caption layout available").layout;
        let text_height = f64::from(layout.height()).min((bottom - frame.y0 - 12.0).max(0.0));
        if text_height < f64::from(size) {
            return;
        }
        let x = frame.center().x - f64::from(width) / 2.0;
        let y = bottom - text_height - 6.0;
        let backdrop = Rect::new(x - 7.0, y - 4.0, x + f64::from(width) + 7.0, bottom);
        painter.with_fill_clip(RoundedRect::from_rect(frame, 8.0), |painter| {
            painter
                .fill(
                    RoundedRect::from_rect(backdrop, 5.0),
                    Color::BLACK.with_alpha(0.82),
                )
                .draw();
            painter.with_fill_clip(
                Rect::new(x, y, x + f64::from(width), y + text_height),
                |painter| {
                    render_text(
                        painter,
                        Affine::translate((x, y)),
                        layout,
                        &[Color::WHITE.into()],
                        true,
                    );
                },
            );
        });
    });
}
