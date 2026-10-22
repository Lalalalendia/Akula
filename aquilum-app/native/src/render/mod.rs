pub(crate) mod damage;
pub mod fade;
mod layer;
pub mod live;
mod mark;
mod offscreen;
#[cfg_attr(feature = "installed", path = "overlay_off.rs")]
mod overlay;
mod paint;
mod pixels;
pub mod scroll;
mod renderer;
pub mod sprite;
mod stamp;
mod surface;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod colr_bounds_tests;

pub use offscreen::offscreen;
pub use pixels::{brush, from_rgba};
pub use renderer::Renderer;
#[cfg(not(feature = "installed"))]
pub use renderer::StageSample;
pub use surface::Surfaces;
