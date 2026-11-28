use super::{Engine, SEEK_SETTLE};
use masonry::peniko::ImageBrush;
use std::path::Path;
use std::time::Instant;

#[derive(Default)]
pub(crate) struct Playback {
    engine: Option<Engine>,
    frame: Option<ImageBrush>,
    playing: bool,
    position: f64,
    seek: Option<f64>,
    until: Option<Instant>,
}

impl Playback {
    pub(crate) fn open(&mut self, path: &Path) -> bool {
        if self.engine.is_none() {
            self.engine = Engine::open(path);
            self.until = Some(Instant::now() + SEEK_SETTLE);
        }
        self.engine.is_some()
    }

    pub(crate) fn seek(&mut self, ms: i64) {
        self.seek = Some(ms as f64 / 1000.0);
        self.until = Some(Instant::now() + SEEK_SETTLE);
    }

    pub(crate) fn toggle(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.toggle();
            self.playing = engine.status().0;
        }
    }

    pub(crate) fn tick(&mut self) -> bool {
        let Some(engine) = self.engine.as_mut() else {
            return false;
        };
        let (playing, position, duration, _) = engine.status();
        self.playing = playing;
        self.position = position;
        if duration > 0.0
            && let Some(target) = self.seek.take()
        {
            let fraction = (target / duration).clamp(0.0, 1.0);
            #[cfg(target_os = "linux")]
            engine.seek_exact(fraction);
            #[cfg(not(target_os = "linux"))]
            engine.seek(fraction);
            self.until = Some(Instant::now() + SEEK_SETTLE);
        }
        if let Some(frame) = engine.frame() {
            self.frame = Some(frame);
        }
        let settling = self.until.is_some_and(|end| Instant::now() < end);
        if !settling {
            self.seek = None;
        }
        playing || settling
    }

    pub(crate) fn frame(&self) -> Option<&ImageBrush> {
        self.frame.as_ref()
    }

    pub(crate) fn is_open(&self) -> bool {
        self.engine.is_some()
    }

    pub(crate) fn position(&self) -> f64 {
        self.position
    }
}
