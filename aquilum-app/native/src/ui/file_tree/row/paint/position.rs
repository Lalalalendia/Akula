use crate::ui::tokens::size;

pub(super) fn icon_x(depth: usize) -> f64 {
    size::SIDEBAR_ITEM_PADDING + depth as f64 * size::SIDEBAR_TREE_INDENT
}

pub(super) fn text_x(depth: usize) -> f64 {
    icon_x(depth) + size::SIDEBAR_ICON_SIZE + size::SIDEBAR_ITEM_GAP
}
