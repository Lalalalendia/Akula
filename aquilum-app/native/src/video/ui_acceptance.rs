//! VIDEO-16 non-shipping GUI acceptance seam. Never part of the default app.
//! A real OS click still passes through the normal editor, queue, storage,
//! pinned yt-dlp/QuickJS and Rust mux; only its network transport is localhost.
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

const TEST_ID: &str = "dQw4w9WgXcQ";

// An observation-only native cursor coordinate. The GUI test still sends
// real Windows pointer input; this never invokes a click inside Akula.
#[repr(C)]
struct NativePoint { x: i32, y: i32 }
#[link(name = "user32")]
unsafe extern "system" {
    fn GetCursorPos(point: *mut NativePoint) -> i32;
}
fn cursor() -> Option<(i32, i32)> {
    let mut point = NativePoint { x: 0, y: 0 };
    // SAFETY: Win32 writes to a live repr(C) point supplied by this function.
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

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
    record(event, None);
}

// The caller supplies the target-button center in editor logical coordinates,
// relative to the current pointer. Only non-shipping Windows GUI CI uses this.
fn target_pixel(source: (i32, i32), delta: (f64, f64)) -> Option<(i32, i32)> {
    let (dx, dy) = delta;
    if !dx.is_finite() || !dy.is_finite() || dx.abs() > 64.0 || dy.abs() > 64.0 {
        return None;
    }
    Some(((source.0 as f64 + dx).round() as i32,
          (source.1 as f64 + dy).round() as i32))
}

fn record(event: &'static str, center_delta: Option<(f64, f64)>) {
    if !hosted() || !matches!(event,
        "HOVER_DOWNLOAD" | "CLICK_DOWNLOAD" | "HOVER_LOCAL_PLAY" |
        "CLICK_LOCAL_PLAY_ATTEMPT" | "CLICK_LOCAL_PLAY" | "DECODED_FRAME")
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
    // Never accept a missing/invalid target for hover events: a random edge
    // pixel must not stand in for the actual Download/Play button center.
    let hover_point = if event.starts_with("HOVER_") {
        cursor().zip(center_delta).and_then(|(source, delta)|
            target_pixel(source, delta).map(|target| (source, target)))
    } else { None };
    if event.starts_with("HOVER_") && hover_point.is_none() { return; }
    static SEEN: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut seen = SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if seen.contains(event) { return; }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let recorded = if let Some(((x, y), (tx, ty))) = hover_point {
            writeln!(file, "VIDEO16_{event}_SOURCE:{x}:{y}\nVIDEO16_{event}_POINT:{tx}:{ty}\nVIDEO16_{event}").is_ok()
        } else {
            writeln!(file, "VIDEO16_{event}").is_ok()
        };
        if recorded { seen.insert(event); }
    }
}

pub(crate) fn blank_preview() -> crate::ui::editor::resolve::Fetch<crate::ui::editor::resolve::Media> {
    use crate::ui::editor::resolve::{Fetch, Media};
    Fetch::Ready(Media { image: None, natural: (0, 0), video: true, path: None })
}

pub(crate) fn frame_ready(ready: bool) {
    if ready { mark("DECODED_FRAME"); }
}

pub(crate) fn hover(download: bool, play: bool, local: bool, center_delta: Option<(f64, f64)>) {
    if download { record("HOVER_DOWNLOAD", center_delta); }
    if play && local { record("HOVER_LOCAL_PLAY", center_delta); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_center_delta_rejects_unsafe_or_nonfinite_offsets() {
        assert_eq!(target_pixel((400, 300), (11.5, -5.0)), Some((412, 295)));
        assert_eq!(target_pixel((400, 300), (65.0, 0.0)), None);
        assert_eq!(target_pixel((400, 300), (f64::NAN, 0.0)), None);
    }

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
