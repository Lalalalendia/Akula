mod notice;
use super::{JobId, PathBuf};
use aquilum_core::background_jobs::State;
pub(crate) use notice::{BatchReport, Notice};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Track {
    pub(crate) index: u32,
    pub(crate) language: Option<String>,
    pub(crate) title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Selection {
    pub(crate) source: PathBuf,
    pub(crate) tracks: Vec<Track>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JobRow {
    pub(crate) id: JobId,
    pub(crate) source: PathBuf,
    pub(crate) state: State,
    pub(crate) committed: i64,
    pub(crate) total: Option<i64>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct View {
    pub(crate) tools: [Option<PathBuf>; 5],
    pub(crate) jobs: Vec<JobRow>,
    pub(crate) selection: Option<Selection>,
    pub(crate) active: Option<JobId>,
    pub(crate) active_source: Option<PathBuf>,
    pub(crate) processing_queue: bool,
    pub(crate) has_queued: bool,
    pub(crate) caption_job: Option<JobId>,
    pub(crate) preparing: bool,
    pub(crate) installing_tools: Option<u8>,
    pub(crate) auto_downloads: bool,
    pub(crate) admitting_download: Option<PathBuf>,
    pub(crate) loaded: bool,
    pub(crate) older: bool,
    pub(crate) newer: bool,
    pub(crate) notice: Option<Notice>,
}

impl View {
    pub(crate) fn ready(&self) -> bool {
        self.loaded
            && !self.preparing
            && self.installing_tools.is_none()
            && self.active.is_none()
            && self.tools.iter().all(Option::is_some)
    }
}
