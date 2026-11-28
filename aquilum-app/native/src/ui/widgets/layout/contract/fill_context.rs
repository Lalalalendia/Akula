//! Measure the full available length, or zero if the context has no length.
macro_rules! fill_context {
    () => {
        fn measure(&mut self, ctx: &mut masonry::core::MeasureCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, axis: masonry::kurbo::Axis, _len_req: masonry::layout::LenReq, _cross: Option<masonry::layout::Length>) -> masonry::layout::Length {
            ctx.context_size().length(axis).unwrap_or(masonry::layout::Length::ZERO)
        }
    };
}
pub(in crate::ui) use fill_context;
