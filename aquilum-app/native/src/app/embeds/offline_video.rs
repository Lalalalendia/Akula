//! Native device-only video resolution. A synced note carries only its URL;
//! each device independently resolves a verified local file by video ID.
use super::{Inner, media};
use crate::ui::editor::resolve::{Fetch, Media};

pub(super) fn existing(inner: &Inner, id: &str) -> Option<Fetch<Media>> {
    let app_data = inner.cache.parent()?;
    let library = crate::video::library::Library::new(app_data, &inner.core.known_roots()).ok()?;
    let path = library.existing(id).ok()??;
    let picture = media::poster(&path);
    let natural = picture.as_ref().map_or((0, 0), |p| p.natural);
    Some(Fetch::Ready(Media {
        image: picture.map(|p| p.image), natural,
        video: true, path: Some(path),
    }))
}
