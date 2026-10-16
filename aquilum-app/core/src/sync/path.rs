use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SyncPathError {
    InvalidPath,
    ReservedName,
    Collision { first: String, second: String },
    Io(String),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct SyncRelativePath(String);

impl SyncRelativePath {
    pub fn parse(input: &str) -> Result<Self, SyncPathError> {
        if input.is_empty() || input.starts_with('/') || input.contains('\\')
            || input.contains(':') || input.chars().any(|ch| {
                ch.is_control() || matches!(ch, '*' | '?' | '"' | '<' | '>' | '|')
            })
        {
            return Err(SyncPathError::InvalidPath);
        }
        let mut parts = Vec::new();
        for part in input.split('/') {
            if part.is_empty() || part == "." || part == ".."
                || part.ends_with('.') || part.ends_with(' ')
            {
                return Err(SyncPathError::InvalidPath);
            }
            let normalized: String = part.nfc().collect();
            let base = normalized.split('.').next().unwrap_or("").to_ascii_uppercase();
            if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (base.len() == 4
                    && (base.starts_with("COM") || base.starts_with("LPT"))
                    && matches!(base.as_bytes()[3], b'1'..=b'9'))
            {
                return Err(SyncPathError::ReservedName);
            }
            parts.push(normalized);
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn collision_key(&self) -> String {
        self.0.nfc().flat_map(char::to_lowercase).collect()
    }
}

/// Portable collision policy is deliberately stricter than a case-sensitive local filesystem.
pub fn reject_path_collisions<'a>(
    paths: impl IntoIterator<Item = &'a SyncRelativePath>,
) -> Result<(), SyncPathError> {
    let mut known = BTreeMap::new();
    for path in paths {
        let key = path.collision_key();
        if let Some(first) = known.insert(key, path.as_str().to_owned()) {
            return Err(SyncPathError::Collision {
                first,
                second: path.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

/// Existing filesystem entries must not be symlinks. This is a preflight check,
/// not a race-free write authorization: PR 04 must guard the final file operation.
pub fn resolve_no_symlinks(root: &Path, relative: &SyncRelativePath) -> Result<PathBuf, SyncPathError> {
    let canonical_root = fs::canonicalize(root).map_err(|e| SyncPathError::Io(e.to_string()))?;
    if !canonical_root.is_dir() {
        return Err(SyncPathError::InvalidPath);
    }
    let mut target = canonical_root.clone();
    for component in Path::new(relative.as_str()).components() {
        let Component::Normal(name) = component else {
            return Err(SyncPathError::InvalidPath);
        };
        target.push(name);
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(SyncPathError::InvalidPath);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(SyncPathError::Io(error.to_string())),
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_unicode_and_reject_unsafe_components() {
        let composed = SyncRelativePath::parse("notes/caf\u{e9}.md").unwrap();
        let decomposed = SyncRelativePath::parse("notes/cafe\u{301}.md").unwrap();
        assert_eq!(composed, decomposed);
        for invalid in ["", "/absolute", "../x", "a/../b", "./x", "a//b",
            "a\\b", "C:/foo", "folder/.","folder/..", "file.", "folder /x",
            "a\n.md", "a\0b", "foo:bar"] {
            assert!(SyncRelativePath::parse(invalid).is_err(), "{invalid:?}");
        }
        for invalid in ["CON", "aux.txt", "com1.md", "LPT9", "NUL.tmp"] {
            assert_eq!(SyncRelativePath::parse(invalid), Err(SyncPathError::ReservedName));
        }
    }

    #[test]
    fn detects_case_and_unicode_collisions() {
        let a = SyncRelativePath::parse("Notes/Caf\u{e9}.md").unwrap();
        let b = SyncRelativePath::parse("notes/cafe\u{301}.md").unwrap();
        assert!(matches!(reject_path_collisions([&a, &b]), Err(SyncPathError::Collision { .. })));
    }

    #[test]
    fn refuses_symlink_traversal() {
        let root = tempfile::tempdir().unwrap();
        let target = SyncRelativePath::parse("new/note.md").unwrap();
        assert_eq!(resolve_no_symlinks(root.path(), &target).unwrap(),
            root.path().canonicalize().unwrap().join("new/note.md"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let external = tempfile::tempdir().unwrap();
            symlink(external.path(), root.path().join("escape")).unwrap();
            let attempt = SyncRelativePath::parse("escape/file.md").unwrap();
            assert_eq!(resolve_no_symlinks(root.path(), &attempt), Err(SyncPathError::InvalidPath));
        }
    }
}
