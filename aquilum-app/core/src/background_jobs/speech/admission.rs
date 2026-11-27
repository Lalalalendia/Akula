use super::{
    Error, JobId, KIND, Queue, Recipe, Result, Tools, Workspace, fingerprint, local_file, media,
};
use crate::background_jobs::Spec;
use std::path::{Path, PathBuf};

pub const MAX_BATCH_FILES: usize = 32;

pub fn enqueue(
    queue: &Queue,
    tools: &Tools,
    source: &Path,
    track: Option<u32>,
    language: &str,
) -> Result<JobId> {
    admit(queue, tools, source, track, |track| {
        tools.recipe(track, language, &mut || Ok(()))
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
            admit(queue, tools, source, None, |track| {
                Ok(Recipe {
                    track,
                    ..template.clone()
                })
            })
        })
        .collect())
}

fn admit(
    queue: &Queue,
    tools: &Tools,
    source: &Path,
    track: Option<u32>,
    recipe: impl FnOnce(u32) -> Result<Recipe>,
) -> Result<JobId> {
    let source = local_file(source)?;
    let source_name = source
        .to_str()
        .ok_or_else(|| Error::Invalid("media path is not representable as UTF-8".into()))?;
    let mut check = || Ok(());
    let input = fingerprint(&source, &mut check)?;
    let probe = media::probe(&tools.ffprobe, &source, &Workspace::new()?, &mut check)?;
    let track = match track {
        Some(index) => index,
        None => {
            probe
                .tracks
                .first()
                .ok_or_else(|| Error::Invalid("media has no audio tracks".into()))?
                .index
        }
    };
    probe.duration_ms(track)?;
    let recipe = recipe(track)?;
    input.verify(&source)?;
    Ok(queue.enqueue(&Spec {
        kind: KIND.into(),
        source: source_name.into(),
        revision: input.digest,
        recipe: serde_json::to_string(&recipe)?,
    })?)
}
