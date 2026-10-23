pub(in crate::app) enum Effect {
    None,
    Rescale,
    Panel,
    Content,
    Restyle(bool),
    Everything,
}
