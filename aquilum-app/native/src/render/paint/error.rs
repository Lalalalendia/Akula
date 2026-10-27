#[derive(Debug)]
pub enum PaintError {
    Unbalanced(&'static str),
    FilterBudget(&'static str),
    GeometryBudget(&'static str),
    ViewportBudget(&'static str),
}
mod display;
