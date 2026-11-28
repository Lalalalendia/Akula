use super::{Command, Dispatcher, JobId};
use aquilum_core::background_jobs::speech::{edits, playback::Transcript};
use std::sync::mpsc::{self, Receiver, Sender};

pub(crate) enum Task {
    Page(i64),
    Save(edits::CueId, i64, String),
    VerifyMedia, Locate(aquilum_core::background_jobs::speech::search::Target),
}

pub(crate) enum Reply {
    Page(edits::Page),
    Saved(edits::Row),
    Media(Transcript), Located(edits::Page, Transcript, i64),
}

pub(crate) type Response = Result<Reply, String>;

pub(crate) struct Request {
    pub job: JobId,
    pub task: Task,
    pub reply: Sender<Response>,
}

impl Dispatcher {
    pub(crate) fn transcript(&self, job: JobId, task: Task) -> Receiver<Response> {
        let (reply, receiver) = mpsc::channel();
        self.send(Command::Transcript(Request { job, task, reply }));
        receiver
    }
}
