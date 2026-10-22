use masonry::core::{MeasureCtx, Widget};
use masonry::layout::{LayoutSize, LenReq, Length};
use masonry::kurbo::Axis;
use crate::ui::tokens::size;
use super::area::ScrollArea;

impl<W: Widget> ScrollArea<W> {
    pub(super) fn measure_cross(&mut self, ctx: &mut MeasureCtx<'_>, cross: Option<Length>) -> Length {
        let content = ctx.compute_length(&mut self.child, LenReq::MaxContent.into(), LayoutSize::NONE, Axis::Horizontal, None);
        let height = ctx.compute_length(&mut self.child, LenReq::MaxContent.into(), LayoutSize::NONE, Axis::Vertical, Some(content));
        let overflow = cross.is_some_and(|width| content.get() > width.get() + 0.5);
        Length::px(height.get() + if overflow { size::SCROLLBAR_WIDTH } else { 0.0 })
    }
}

