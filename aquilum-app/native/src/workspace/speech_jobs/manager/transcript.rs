use super::Manager;
use crate::workspace::speech_jobs::transcript::{Reply, Request, Response, Task};
use aquilum_core::background_jobs::speech::{edits, playback::Transcript};

impl Manager<'_> {
    pub(super) fn transcript(&mut self, request: Request) {
        let result = self.transcript_result(request.job, request.task);
        if request.reply.send(result).is_ok() {
            (self.wake)();
        }
    }

    fn transcript_result(&mut self, job: i64, task: Task) -> Response {
        match task {
            Task::Page(number) => edits::page(&self.queue, job, number).map(Reply::Page),
            Task::VerifyMedia => Transcript::verify(&self.queue, job).map(Reply::Media),
            Task::Save(cue, revision, text) => edits::save(&self.queue, job, cue, revision, &text)
                .map(|row| {
                    if let Some(track) = self
                        .shared
                        .playback
                        .selected()
                        .filter(|track| track.id() == job)
                        && let Some(index) = track.requested()
                    {
                        self.caption_update(track.load(&self.queue, index));
                    }
                    Reply::Saved(row)
                }),
        }
        .map_err(|error| error.to_string())
    }
}
