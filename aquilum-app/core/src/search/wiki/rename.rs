use super::super::error::SearchError;
use super::parser::extract;
use super::resolver::LinkResolver;
use super::target::{relative_key, relative_target, title_key};
use crate::files::document::{hash_bytes, normalize_line_endings};
use crate::search::paths::same_path;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

mod target_path;
use target_path::targets_path;

pub struct LinkRewrite {
    pub path: PathBuf,
    pub original: String,
    pub original_hash: String,
    pub content: String,
}

pub fn plan_for_rename(
    connection: &Connection,
    root: &Path,
    old_path: &Path,
    new_path: &Path,
    sources: Vec<PathBuf>,
    fallback_candidates: Option<&[String]>,
) -> Result<Vec<LinkRewrite>, SearchError> {
    let mut rewrites = Vec::new();
    let mut resolver = LinkResolver::new(connection)?;
    // Values below are independent of the referencing document. Compute
    // them once for a bulk rename, not once per matching wiki link.
    let old_relative_key = relative_key(root, old_path);
    let old_title_key = title_key(old_path);
    let new_relative_target = relative_target(root, new_path, true);
    let new_title_target = relative_target(root, new_path, false);
    for stored_path in sources {
        let source_path = if same_path(&stored_path, old_path) {
            new_path
        } else {
            &stored_path
        };
        let Ok(original) = fs::read_to_string(source_path) else {
            continue;
        };
        // Without an opener extract() cannot return any links. Avoid a
        // second copy, line-ending normalization, and Markdown parsing.
        if !original.contains("[[") {
            continue;
        }
        let text = normalize_line_endings(original.clone());
        let mut changes = Vec::new();
        for link in extract(&text) {
            if targets_path(
                &mut resolver,
                root,
                &stored_path,
                &link.target,
                old_path,
                &old_relative_key,
                &old_title_key,
                fallback_candidates,
            )? {
                let path_style = text[link.note_range.clone()]
                    .bytes()
                    .any(|byte| byte == b'/' || byte == b'\\');
                changes.push((
                    link.note_range,
                    if path_style {
                        new_relative_target.clone()
                    } else {
                        new_title_target.clone()
                    },
                ));
            }
        }
        if changes.is_empty() {
            continue;
        }
        let mut content = text;
        for (range, replacement) in changes.into_iter().rev() {
            content.replace_range(range, &replacement);
        }
        let original_hash = hash_bytes(original.as_bytes());
        rewrites.push(LinkRewrite {
            path: stored_path,
            original,
            original_hash,
            content,
        });
    }
    Ok(rewrites)
}
