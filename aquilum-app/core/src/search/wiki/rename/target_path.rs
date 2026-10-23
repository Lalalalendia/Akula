use super::super::resolver::LinkResolver;
use super::super::target::{select_candidate, target_key};
use crate::search::error::SearchError;
use crate::search::paths::same_path;
use std::path::Path;

pub(super) fn targets_path(
    resolver: &mut LinkResolver<'_>,
    root: &Path,
    source: &Path,
    target: &str,
    old_path: &Path,
    old_relative_key: &str,
    old_title_key: &str,
    fallback_candidates: Option<&[String]>,
) -> Result<bool, SearchError> {
    let (key, kind) = target_key(target);
    if let Some(candidates) = fallback_candidates {
        return Ok(if kind == "path" {
            key == old_relative_key
        } else {
            key == old_title_key
                && select_candidate(root, source, candidates)
                    .is_some_and(|path| same_path(&path, old_path))
        });
    }
    if let Some(path) = resolver.resolve(root, source, target)? {
        return Ok(same_path(&path, old_path));
    }
    Ok(if kind == "path" {
        key == old_relative_key
    } else {
        key == old_title_key
    })
}
