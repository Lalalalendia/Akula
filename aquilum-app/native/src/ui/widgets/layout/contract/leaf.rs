// Only widgets with no child and no layout work may use this contract.
macro_rules! leaf_widget {
    () => {
        fn register_children(&mut self, _ctx: &mut masonry::core::RegisterCtx<'_>) {}
        fn layout(&mut self, _ctx: &mut masonry::core::LayoutCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, _size: masonry::kurbo::Size) {}
        fn children_ids(&self) -> masonry::core::ChildrenIds {
            masonry::core::ChildrenIds::new()
        }
    };
}
pub(in crate::ui) use leaf_widget;
