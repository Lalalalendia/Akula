use crate::ui::widgets::layout::contract::passive_single_child;
use masonry::core::{
    LayoutCtx, MeasureCtx, NewWidget, PropertiesMut, PropertiesRef, Update, UpdateCtx, Widget, WidgetMut,
    WidgetPod,
};
use masonry::kurbo::{Axis, Point, Size};
use masonry::layout::{LenReq, Length};

pub struct Visible {
    child: WidgetPod<dyn Widget>,
    shown: bool,
}

impl Visible {
    pub fn new(child: NewWidget<impl Widget + ?Sized>, shown: bool) -> NewWidget<Self> {
        NewWidget::new(Visible { child: child.erased().to_pod(), shown })
    }

    pub fn set_shown(this: &mut WidgetMut<'_, Self>, shown: bool) {
        if this.widget.shown != shown {
            this.widget.shown = shown;
            this.ctx.set_stashed(&mut this.widget.child, !shown);
            this.ctx.request_layout();
        }
    }
}

impl Widget for Visible {
    type Action = masonry::core::NoAction;

    fn update(&mut self, ctx: &mut UpdateCtx<'_>, _props: &mut PropertiesMut<'_>, event: &Update) {
        if matches!(event, Update::WidgetAdded) && !self.shown {
            ctx.set_stashed(&mut self.child, true);
        }
    }

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, len_req: LenReq, cross: Option<Length>) -> Length {
        if !self.shown {
            return Length::ZERO;
        }
        let context = ctx.context_size();
        ctx.compute_length(&mut self.child, len_req.into(), context, axis, cross)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        if self.shown {
            ctx.run_layout(&mut self.child, size);
            ctx.place_child(&mut self.child, Point::ORIGIN);
        }
    }

    passive_single_child!(child);
}
