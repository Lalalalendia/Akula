//! Shared transparent container contract, optionally with one registered child.
//! Measurement, layout, update and input handling stay in each widget.
macro_rules! passive_surface {
    ($($child:ident)?) => {
        $(
            fn register_children(&mut self, ctx: &mut masonry::core::RegisterCtx<'_>) {
                ctx.register_child(&mut self.$child);
            }
            fn children_ids(&self) -> masonry::core::ChildrenIds {
                masonry::core::ChildrenIds::from_slice(&[self.$child.id()])
            }
        )?

        fn paint(&mut self, _ctx: &mut masonry::core::PaintCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, _painter: &mut masonry::imaging::Painter<'_>) {}

        fn accessibility_role(&self) -> masonry::accesskit::Role {
            masonry::accesskit::Role::GenericContainer
        }

        fn accessibility(&mut self, _ctx: &mut masonry::core::AccessCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, _node: &mut masonry::accesskit::Node) {}
    };
}
pub(in crate::ui) use {passive_surface, passive_surface as passive_single_child};
