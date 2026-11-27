use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Notice {
    Error(String),
    LeaseBusy, MemoryPressure,
    Exported,
    Batch(BatchReport),
    BatchTooLarge,
    DownloadQueued(PathBuf),
    DownloadError(PathBuf, String),
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct BatchReport {
    pub accepted: usize,
    pub rejected: usize,
    pub example: Option<String>,
}
