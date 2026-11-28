//! Shared, narrow framework contracts; each widget owns its rendering and input.
mod single_child;
mod labeled_click;
mod leaf;
mod fill_context;
pub(in crate::ui) use {
    single_child::{passive_single_child, passive_surface}, labeled_click::labeled_click,
    leaf::leaf_widget, fill_context::fill_context,
};
