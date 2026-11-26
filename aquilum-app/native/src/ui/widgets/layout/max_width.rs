use crate::ui::widgets::layout::contract::passive_single_child;
use masonry::core::{LayoutCtx, MeasureCtx, NewWidget, PropertiesRef, Widget, WidgetPod};
use masonry::kurbo::{Axis, Point, Size};
use masonry::layout::{LayoutSize, LenDef, LenReq, Length, SizeDef};

pub struct MaxWidth<W: Widget> {
    child: WidgetPod<W>,
    max: f64,
}

impl<W: Widget> MaxWidth<W> {
    pub fn new(child: NewWidget<W>, max: f64) -> NewWidget<Self> {
        NewWidget::new(MaxWidth { child: child.to_pod(), max })
    }
}

impl<W: Widget> Widget for MaxWidth<W> {
    type Action = masonry::core::NoAction;

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, len_req: LenReq, cross: Option<Length>) -> Length {
        match axis {
            Axis::Horizontal => match len_req {
                LenReq::MinContent => Length::ZERO,
                LenReq::MaxContent => Length::px(self.max),
                LenReq::FitContent(space) => space,
            },
            Axis::Vertical => {
                let inner = Length::px(cross.map_or(self.max, |c| c.get().min(self.max)));
                let context = LayoutSize::maybe(Axis::Horizontal, Some(inner));
                ctx.compute_length(&mut self.child, LenReq::MaxContent.into(), context, Axis::Vertical, Some(inner))
            }
        }
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        let inner = size.width.min(self.max);
        let auto = SizeDef::new(LenDef::FitContent(Length::px(inner)), LenDef::MaxContent);
        let child = ctx.compute_size(&mut self.child, auto, Size::new(inner, size.height).into());
        let child = Size::new(inner, child.height);
        ctx.run_layout(&mut self.child, child);
        ctx.place_child(&mut self.child, Point::new(((size.width - inner) / 2.0).round(), 0.0));
    }

    passive_single_child!(child);
}
