use masonry::core::NewWidget;
use masonry::layout::Length;
use masonry::widgets::SizedBox;
use crate::ui::tokens::size;
use super::super::step;

pub(super) fn spacer(rows: usize) -> NewWidget<SizedBox> {
    NewWidget::new(SizedBox::empty().height(Length::px(rows as f64 * step() - size::SPACE_2)))
}
