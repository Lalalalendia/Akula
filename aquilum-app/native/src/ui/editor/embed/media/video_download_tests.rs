use super::{MediaHit, MediaView};
use crate::ui::editor::embed::media::parse;
use crate::ui::editor::resolve::{Fetch, Media};
use std::path::PathBuf;

fn preview(src: &str, local: Option<PathBuf>) -> MediaView {
    let line = parse(src);
    let fetched = Fetch::Ready(Media {
        image: None,
        natural: (0, 0),
        video: true,
        path: local,
    });
    MediaView::new(&fetched, line, 640.0, false)
}

fn download_available(view: &MediaView) -> bool {
    view.toolbar_items().iter().any(|(item, _)| matches!(item, Some(MediaHit::Download)))
}

#[test]
fn youtube_preview_has_explicit_download_action_but_never_autostarts() {
    let url = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
    let remote = preview(url, None);
    assert!(download_available(&remote));
    assert_eq!(remote.line.src, url, "markdown text stays unchanged");
    assert!(remote.path.is_none(), "no automatic download on rendering");
}

#[test]
fn verified_local_copy_replaces_download_action_with_playback() {
    let url = "https://youtu.be/dQw4w9WgXcQ";
    let local = PathBuf::from("device-only/dQw4w9WgXcQ.mp4");
    let installed = preview(url, Some(local.clone()));
    assert!(!download_available(&installed));
    assert_eq!(installed.path.as_ref(), Some(&local));
    assert_eq!(installed.line.src, url);
    let regular = preview("clip.mp4", None);
    assert!(!download_available(&regular), "only YouTube cards request yt-dlp");
}
