use super::{Failure, JobId, Outcome, speech};
use crate::workspace::speech_jobs::config::Config;
use aquilum_core::background_jobs::Queue;
use std::path::Path;

pub(super) fn run(
    database: &Path,
    config: &Config,
    id: JobId,
    queued_only: bool,
) -> Result<Outcome, Failure> {
    let mut queue = Queue::open(database).map_err(|error| Failure::Other(error.to_string()))?;
    let tools = config.verify().map_err(Failure::Other)?;
    let threads = config.cpu_mode.threads();
    let work = if queued_only {
        speech::work_queued
    } else {
        speech::work
    };
    work(&mut queue, &tools, id, threads).map_err(|error| match error {
        speech::Error::ResourcePressure => Failure::MemoryPressure,
        other => Failure::Other(other.to_string()),
    })
}
