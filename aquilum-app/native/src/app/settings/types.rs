pub(super) enum Effect {
    None,
    Rescale,
    Panel,
    Content,
    Restyle(bool),
    Everything,
}
