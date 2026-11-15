//! Thread-safe invalidation of video cards after download progress/completion.
use std::sync::atomic::Ordering;
use super::Embeds;

impl Embeds {
    pub fn video_update(&self, source: &str, finished: bool) {
        if finished {
            // A finished local download changes the Media.path from None
            // (remote thumbnail) to the verified local file. A stale remote
            // resolution must not put the old preview back after completion.
            self.0.media.epoch.fetch_add(1, Ordering::AcqRel);
            self.0.media.entries.lock().unwrap_or_else(|e| e.into_inner()).remove(source);
        }
        self.0.touch();
    }
}
