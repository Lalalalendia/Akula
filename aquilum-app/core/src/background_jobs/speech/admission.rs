mod source;
use super::{Error, JobId, Queue, Recipe, Result, Tools};
use crate::background_jobs::Spec;
use source::spec;
use std::path::{Path, PathBuf};

pub const MAX_BATCH_FILES: usize = 32;

pub fn enqueue(
    queue: &Queue,
    tools: &Tools,
    source: &Path,
    track: Option<u32>,
    language: &str,
) -> Result<JobId> {
    Ok(queue.enqueue(&prepare_enqueue(
        tools,
        source,
        track,
        language,
        &mut || Ok(()),
    )?)?)
}

pub fn prepare_enqueue(
    tools: &Tools,
    source: &Path,
    track: Option<u32>,
    language: &str,
    check: &mut impl FnMut() -> Result<()>,
) -> Result<Spec> {
    spec(tools, source, track, check, |track, check| {
        tools.recipe(track, language, check)
    })
}

/// The tool recipe is fingerprinted once. Each result comes from an independent
/// admission, so an unreadable file cannot prevent the remaining files from queuing.
pub fn enqueue_batch(
    queue: &Queue,
    tools: &Tools,
    sources: &[PathBuf],
    language: &str,
) -> Result<Vec<Result<JobId>>> {
    if sources.len() > MAX_BATCH_FILES {
        return Err(Error::Invalid("too many files in one speech batch".into()));
    }
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let template = tools.recipe(0, language, &mut || Ok(()))?;
    Ok(sources
        .iter()
        .map(|source| {
            let spec = spec(tools, source, None, &mut || Ok(()), |track, _| {
                Ok(Recipe {
                    track,
                    ..template.clone()
                })
            })?;
            Ok(queue.enqueue(&spec)?)
        })
        .collect())
}
