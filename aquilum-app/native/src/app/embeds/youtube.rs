//! Thumbnail-only single-video preview. No iframe or video download.
use super::{Inner, media};
use crate::ui::editor::resolve::{Fetch, Media};

pub(super) fn preview(inner: &Inner, source: &str, wiki: bool) -> Option<Fetch<Media>> {
    let id = (!wiki).then(|| crate::video::youtube::video_id(source)).flatten()?;
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
