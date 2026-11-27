use super::super::Tool;
use super::*;

impl Manager<'_> {
    pub(super) fn prepare(
        &mut self,
        action: impl FnOnce(&mut Self) -> Result<(), String>,
    ) -> Result<(), String> {
        if self.worker.is_some() {
            return Err("a speech job is already active".into());
        }
        self.view.preparing = true;
        self.publish();
        let result = action(self);
        self.view.preparing = false;
        result
    }

    pub(super) fn inspect(&mut self, source: PathBuf) -> Result<(), String> {
        self.view.selection = None;
        let probe = self.config.checked(Tool::Probe)?;
        let tracks = speech::tracks(probe, &source).map_err(|error| error.to_string())?;
        if tracks.is_empty() {
            return Err("the media file has no audio tracks".into());
        }
        let tracks = tracks
            .into_iter()
            .map(|track| {
                let (language, title) = match track.tags {
                    Some(tags) => (tags.language, tags.title),
                    None => (None, None),
                };
                Track {
                    index: track.index,
                    language,
                    title,
                }
            })
            .collect();
        self.view.selection = Some(Selection { source, tracks });
        Ok(())
    }

    pub(super) fn add(&mut self, track: u32) -> Result<(), String> {
        let selection = self
            .view
            .selection
            .as_ref()
            .ok_or("select a media file first")?;
        if !selection
            .tracks
            .iter()
            .any(|candidate| candidate.index == track)
        {
            return Err("selected audio track is unavailable".into());
        }
        let tools = self.config.verify()?;
        speech::enqueue(&self.queue, &tools, &selection.source, Some(track), "auto")
            .map_err(|error| error.to_string())?;
        self.view.selection = None;
        self.bounds.clear();
        Ok(())
    }
}
