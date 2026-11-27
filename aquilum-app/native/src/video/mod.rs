#[cfg(windows)] pub(crate) mod optional_ytdlp;
pub(crate) mod captions; pub(crate) mod download; mod worker_process;
pub(crate) mod library;
pub(crate) mod light;
pub(crate) mod queue; mod queue_dispatch;
pub(crate) mod youtube; #[cfg(any(test, all(windows, feature = "installed")))] #[allow(dead_code)] mod tools_package;
#[cfg(all(windows, any(test, feature = "installed")))] #[allow(dead_code)] pub(crate) mod tools_package_probe; #[cfg(all(windows, any(test, feature = "installed")))] #[allow(dead_code)] mod tools_package_pin_admission; #[cfg(all(windows, any(test, feature = "installed")))] #[allow(dead_code)] mod tools_package_01h_process_boundary_windows;
#[cfg(all(windows, any(feature = "video-ui-e2e", feature = "video-tools-gui-e2e")))] pub(crate) mod ui_acceptance;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
mod stub;
#[cfg(target_os = "windows")]
mod windows;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use masonry::peniko::ImageBrush;
#[cfg(target_os = "linux")]
use self::linux::Engine;
#[cfg(target_os = "macos")]
use self::macos::Engine;
#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
use self::stub::Engine;
#[cfg(target_os = "windows")]
use self::windows::Engine;
#[cfg(target_os = "linux")]
pub fn poster(path: &Path) -> Option<ImageBrush> {
    Engine::poster(path)
}
#[cfg(target_os = "macos")]
pub use self::macos::poster;
const MOVED: f64 = 0.05;
const SEEK_SETTLE: std::time::Duration = std::time::Duration::from_secs(2);
#[derive(Clone, Debug, Default)]
pub struct State {
    pub frame: Option<ImageBrush>,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub muted: bool,
}
struct Player {
    engine: Engine,
    state: State,
    source: Option<aquilum_core::background_jobs::speech::playback::MediaSource>,
    caption: Option<std::sync::Arc<str>>,
    seek_until: Option<std::time::Instant>,
}
impl Player {
    fn tick(&mut self) -> bool {
        let fresh = self.engine.frame();
        let changed = fresh.is_some();
        if let Some(frame) = fresh {
            self.state.frame = Some(frame);
        }
        let (playing, position, duration, muted) = self.engine.status();
        let moved = (position - self.state.position).abs() > MOVED || playing != self.state.playing || muted != self.state.muted;
        self.state = State { frame: self.state.frame.take(), playing, position, duration, muted };
        let cue = captions::for_source(self.source.as_ref(), position);
        let caption_changed = self.caption != cue;
        self.caption = cue;
        changed || moved || caption_changed
    }
}

thread_local! {
    static PLAYERS: RefCell<HashMap<PathBuf, Player>> = RefCell::new(HashMap::new());
}

pub fn state(path: &Path) -> Option<State> {
    PLAYERS.with(|p| p.borrow().get(path).map(|player| player.state.clone()))
}

pub fn toggle(path: &Path) -> bool {
    PLAYERS.with(|players| {
        let mut players = players.borrow_mut();
        if !players.contains_key(path) {
            let source = aquilum_core::background_jobs::speech::playback::MediaSource::open(path).ok();
            let Some(engine) = Engine::open(path) else { return false };
            let source = source.filter(|media| media.is_current());
            players.insert(path.to_path_buf(), Player { engine, state: State::default(), source, caption: None, seek_until: None });
        }
        if let Some(player) = players.get_mut(path) {
            player.engine.toggle();
            player.tick();
        }
        true
    })
}

pub fn seek(path: &Path, fraction: f64) {
    PLAYERS.with(|players| {
        if let Some(player) = players.borrow_mut().get_mut(path) {
            player.engine.seek(fraction);
            player.seek_until = Some(std::time::Instant::now() + SEEK_SETTLE);
            player.tick();
        }
    });
}

pub fn mute(path: &Path) {
    PLAYERS.with(|players| {
        if let Some(player) = players.borrow_mut().get_mut(path) {
            player.engine.mute();
            player.tick();
        }
    });
}

pub fn forget(paths: &[PathBuf]) {
    let _ = PLAYERS.try_with(|players| players.borrow_mut().retain(|path, _| !paths.contains(path)));
}

#[derive(Default)]
pub struct Owned(Vec<PathBuf>);

impl Owned {
    pub fn add(&mut self, path: PathBuf) {
        if !self.has(&path) {
            self.0.push(path);
        }
    }

    pub fn has(&self, path: &Path) -> bool {
        self.0.iter().any(|p| p == path)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn tick(&self) -> (bool, bool) {
        PLAYERS.with(|players| {
            let mut players = players.borrow_mut();
            let (mut changed, mut playing) = (false, false);
            for path in &self.0 {
                if let Some(player) = players.get_mut(path) {
                    changed |= player.tick();
                    playing |= player.state.playing || player.seek_until.is_some_and(|end| std::time::Instant::now() < end);
                }
            }
            (changed, playing)
        })
    }

    pub fn stop(&mut self) {
        let paths = std::mem::take(&mut self.0);
        let _ = PLAYERS.try_with(|players| {
            let mut players = players.borrow_mut();
            for path in &paths {
                players.remove(path);
            }
        });
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
#[path = "playback_tests.rs"]
mod tests;
