//! Thumbnail-only single-video preview. No iframe or video download.
use super::{Inner, media};
use crate::ui::editor::resolve::{Fetch, Media};

pub(super) fn preview(inner: &Inner, source: &str, wiki: bool) -> Option<Fetch<Media>> {
    let id = (!wiki).then(|| crate::video::youtube::video_id(source)).flatten()?;
    // Prefer a verified per-device media file, independent of any vault.
    let local = inner.cache.parent()
        .and_then(|app_data| crate::video::library::Library::new(
            app_data, &inner.core.known_roots()
        ).ok())
        .and_then(|lib| lib.existing(id).ok().flatten());
    if let Some(path) = local {
        let picture = media::poster(&path);
        let natural = picture.as_ref().map_or((0, 0), |p| p.natural);
        return Some(Fetch::Ready(Media {
            image: picture.map(|p| p.image), natural,
            video: true, path: Some(path),
        }));
    }
    // Never fetch an arbitrary host from the pasted Markdown. The existing
    // media cache enforces a compressed-input byte budget.
    let picture = crate::video::youtube::thumbnail_url(id)
        .and_then(|url| media::download(inner, &url))
        .and_then(|path| crate::images::picture(&path));
    let natural = picture.as_ref().map_or((0, 0), |p| p.natural);
    Some(Fetch::Ready(Media {
        image: picture.map(|p| p.image), natural,
        video: true, path: None,
    }))
}
