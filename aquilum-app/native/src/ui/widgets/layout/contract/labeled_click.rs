//! Accessible label and click action, independent of input handling and paint.
//! The caller supplies the exact role (buttons are not links).
macro_rules! labeled_click {
    ($role:ident, $label:ident) => {
        fn accessibility_role(&self) -> masonry::accesskit::Role {
            masonry::accesskit::Role::$role
        }
        fn accessibility(&mut self, _ctx: &mut masonry::core::AccessCtx<'_>, _props: &masonry::core::PropertiesRef<'_>, node: &mut masonry::accesskit::Node) {
            node.set_label(self.$label.clone());
            node.add_action(masonry::accesskit::Action::Click);
        }
    };
}
pub(in crate::ui) use labeled_click;
