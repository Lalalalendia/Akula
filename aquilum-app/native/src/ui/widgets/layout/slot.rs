use crate::ui::widgets::layout::contract::passive_single_child;

#[cfg(test)]
thread_local! {
    // Only the most recent slot replacement matters for the note-page probe.
    static LAST_REPLACE: std::cell::Cell<Option<(std::time::Duration, std::time::Duration, std::time::Duration)>> =
        const { std::cell::Cell::new(None) };
}
use masonry::core::{LayoutCtx, MeasureCtx, NewWidget, PropertiesRef, Widget, WidgetMut, WidgetPod};
use masonry::kurbo::{Axis, Point, Size};
use masonry::layout::{LenReq, Length};

pub struct Slot {
    child: WidgetPod<dyn Widget>,
    fit: bool,
}

impl Slot {
    pub fn new(child: NewWidget<dyn Widget>) -> NewWidget<Self> {
        NewWidget::new(Slot { child: child.to_pod(), fit: false })
    }

    pub fn fit(child: NewWidget<dyn Widget>) -> NewWidget<Self> {
        NewWidget::new(Slot { child: child.to_pod(), fit: true })
    }

    #[cfg(test)]
    pub(crate) fn take_replace_timings() -> Option<(std::time::Duration, std::time::Duration, std::time::Duration)> {
        LAST_REPLACE.with(|last| last.replace(None))
    }

    pub fn set_child(this: &mut WidgetMut<'_, Self>, child: NewWidget<dyn Widget>) {
        #[cfg(test)]
        let before = std::time::Instant::now();
        let pod = child.to_pod();
        #[cfg(test)]
        let converted = std::time::Instant::now();
        let previous = std::mem::replace(&mut this.widget.child, pod);
        this.ctx.remove_child(previous);
        #[cfg(test)]
        let removed = std::time::Instant::now();
        this.ctx.children_changed();
        #[cfg(test)]
        LAST_REPLACE.with(|last| last.set(Some((converted - before, removed - converted, removed.elapsed()))));
    }
}

impl Widget for Slot {
    type Action = masonry::core::NoAction;

    fn measure(&mut self, ctx: &mut MeasureCtx<'_>, _props: &PropertiesRef<'_>, axis: Axis, len_req: LenReq, cross: Option<Length>) -> Length {
        if self.fit {
            let context = ctx.context_size();
            return ctx.compute_length(&mut self.child, len_req.into(), context, axis, cross);
        }
        ctx.context_size().length(axis).unwrap_or(Length::ZERO)
    }

    fn layout(&mut self, ctx: &mut LayoutCtx<'_>, _props: &PropertiesRef<'_>, size: Size) {
        ctx.run_layout(&mut self.child, size);
        ctx.place_child(&mut self.child, Point::ORIGIN);
    }

    passive_single_child!(child);
}
