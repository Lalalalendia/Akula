use super::{JobId, PathBuf, playback};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tool {
    Probe,
    Ffmpeg,
    Whisper,
    Model,
    Vad,
}

impl Tool {
    pub(crate) const ALL: [Self; 5] = [
        Self::Probe,
        Self::Ffmpeg,
        Self::Whisper,
        Self::Model,
        Self::Vad,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Probe => "FFprobe",
            Self::Ffmpeg => "FFmpeg",
            Self::Whisper => "whisper-cli",
            Self::Model => "Whisper",
            Self::Vad => "Silero VAD",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    PickTool(Tool),
    PickMedia, PickBatch,
    UseTrack(u32),
    DismissMedia,
    Run(JobId), RunQueue, StopQueue,
    Pause(JobId),
    Cancel(JobId),
    Export(JobId),
    ShowCaptions(JobId), HideCaptions, OpenTranscript(JobId),
    Refresh,
    Older,
    Newer,
}

pub(crate) enum Command {
    SetTool(Tool, PathBuf),
    Inspect(PathBuf), Batch(Vec<PathBuf>),
    Export(JobId, PathBuf),
    CaptionPage(Arc<playback::Attached>, i64),
    Action(Action), Transcript(super::transcript::Request), Shutdown,
}
