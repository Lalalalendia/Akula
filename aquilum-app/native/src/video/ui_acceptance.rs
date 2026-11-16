//! VIDEO-16 non-shipping GUI acceptance seam. Never part of the default app.
//! A real OS click still passes through the normal editor, queue, storage,
//! pinned yt-dlp/QuickJS and Rust mux; only its network transport is localhost.
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use crate::ui::editor::embed::MediaHit;

const TEST_ID: &str = "dQw4w9WgXcQ";

fn hosted() -> bool {
    std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true")
        && std::env::var("RUNNER_OS").ok().as_deref() == Some("Windows")
        && std::env::var("RUNNER_ENVIRONMENT").ok().as_deref() == Some("github-hosted")
}

pub(crate) fn transport_url(source: &str) -> Result<String, String> {
    if !hosted() || crate::video::youtube::video_id(source) != Some(TEST_ID) {
        return Err("VIDEO-16 refuses non-hosted or unexpected media source".into());
    }
    let url = std::env::var("AKULA_VIDEO16_MPD_URL")
        .map_err(|_| "VIDEO-16 requires explicit localhost MPD".to_owned())?;
    let tail = url.strip_prefix("http://127.0.0.1:")
        .ok_or("VIDEO-16 permits only literal IPv4 loopback HTTP")?;
    let (port, path) = tail.split_once('/').ok_or("VIDEO-16 MPD path missing")?;
    if port.parse::<u16>().ok().filter(|p| *p > 0).is_none() || path != "manifest.mpd" {
        return Err("VIDEO-16 refuses invalid port or non-fixture MPD path".into());
    }
    Ok(url)
}

pub(crate) fn mark(event: &'static str) {
    if !hosted() || !matches!(event,
        "HOVER_DOWNLOAD" | "CLICK_DOWNLOAD" | "HOVER_LOCAL_PLAY" |
        "CLICK_LOCAL_PLAY" | "DECODED_FRAME")
    {
        return;
    }
    let Some(path) = std::env::var_os("AKULA_VIDEO16_PROBE").map(PathBuf::from) else { return };
    let Some(temp) = std::env::var_os("RUNNER_TEMP").map(PathBuf::from) else { return };
    if !path.is_absolute() || !path.starts_with(&temp)
        || path.file_name().and_then(|f| f.to_str()) != Some("video16-events.log")
    {
        return;
    }
    static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if seen.contains(event) { return; }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path)
        && writeln!(file, "VIDEO16_{event}").is_ok()
    {
        seen.insert(event);
    }
}

pub(crate) fn hover(hit: MediaHit, local: bool) {
    match hit {
        MediaHit::Download => mark("HOVER_DOWNLOAD"),
        MediaHit::Play if local => mark("HOVER_LOCAL_PLAY"),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_hosted_loopback_transport_is_eligible() {
        // Developer unit runs have no hosted acceptance environment. This
        // function must never turn a user URL into a remote override.
        if !hosted() {
            assert!(transport_url("https://youtu.be/dQw4w9WgXcQ").is_err());
        }
        assert!(transport_url("https://youtu.be/anotherxxxx").is_err());
    }
}
