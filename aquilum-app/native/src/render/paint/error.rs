#[derive(Debug)]
pub enum PaintError {
    Unbalanced(&'static str),
    FilterBudget(&'static str),
    GeometryBudget(&'static str),
    ViewportBudget(&'static str),
}
impl std::fmt::Display for PaintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaintError::Unbalanced(what) => write!(f, "несбалансированная сцена: {what}"),
            PaintError::FilterBudget(reason) => write!(f, "неподдерживаемая геометрия фильтра: {reason}"),
            PaintError::GeometryBudget(reason) => write!(f, "неподдерживаемая геометрия пути: {reason}"),
            PaintError::ViewportBudget(reason) => write!(f, "неподдерживаемый размер кадра: {reason}"),
        }
    }
}
