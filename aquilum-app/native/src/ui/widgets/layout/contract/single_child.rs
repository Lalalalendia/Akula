//! Identical Widget contract for single-child containers that paint no
//! independent content and expose no independent accessibility action.
//! Measurement, layout, update and input handling stay in each widget.
// A generic, transparent surface with no independent accessibility action.
macro_rules! passive_surface {
    () => {
        fn paint(&mut self, _ctx: &mut masonry::core::PaintCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, _painter: &mut masonry::imaging::Painter<'_>) {}

        fn accessibility_role(&self) -> masonry::accesskit::Role {
            masonry::accesskit::Role::GenericContainer
        }

        fn accessibility(&mut self, _ctx: &mut masonry::core::AccessCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, _node: &mut masonry::accesskit::Node) {}
    };
}
pub(in crate::ui) use passive_surface;

macro_rules! passive_single_child {
    ($child:ident) => {
        fn register_children(&mut self, ctx: &mut masonry::core::RegisterCtx<'_>) {
            ctx.register_child(&mut self.$child);
        }

        $crate::ui::widgets::layout::contract::passive_surface!();

        fn children_ids(&self) -> masonry::core::ChildrenIds {
            masonry::core::ChildrenIds::from_slice(&[self.$child.id()])
        }
    };
}
pub(in crate::ui) use passive_single_child;
