use super::super::{Error, KIND, Recipe, Result, Tools, Workspace, fingerprint, local_file, media};
use crate::background_jobs::Spec;
use std::path::Path;

pub(super) fn spec<C: FnMut() -> Result<()>>(
    tools: &Tools,
    source: &Path,
    track: Option<u32>,
    check: &mut C,
    recipe: impl FnOnce(u32, &mut C) -> Result<Recipe>,
) -> Result<Spec> {
    let source = local_file(source)?;
    let source_name = source
        .to_str()
        .ok_or_else(|| Error::Invalid("media path is not representable as UTF-8".into()))?;
    let input = fingerprint(&source, check)?;
    let probe = media::probe(&tools.ffprobe, &source, &Workspace::new()?, check)?;
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
    let recipe = recipe(track, check)?;
    input.verify(&source)?;
    Ok(Spec {
        kind: KIND.into(),
        source: source_name.into(),
        revision: input.digest,
        recipe: serde_json::to_string(&recipe)?,
    })
}
