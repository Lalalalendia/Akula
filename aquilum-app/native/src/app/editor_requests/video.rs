//! One-click explicit download action; never invoked by preview resolution.
use std::sync::Arc;

use super::App;

impl App {
    pub(super) fn download_video(&mut self, url: &str) {
        let embeds = self.embeds.clone();
        let wake = Arc::clone(&self.workspace.wake);
        let changed: Arc<dyn Fn(bool, &str) + Send + Sync> = Arc::new(move |finished, source| {
            if let Some(embeds) = &embeds {
                embeds.video_update(source, finished);
            } else {
                wake();
            }
            if finished && let Some(crate::video::queue::Status::Failed(reason)) = crate::video::queue::status(source) {
                report!("локальное видео не загружено: {reason}");
            }
        });
        if let Err(reason) = crate::video::queue::enqueue(
            url.to_owned(), self.workspace.data_dir.clone(),
            self.workspace.core.known_roots(), changed,
        ) {
            report!("видеозагрузка отклонена: {reason}");
        }
    }
}
