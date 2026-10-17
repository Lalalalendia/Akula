use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use serde::{Deserialize, Deserializer, Serialize, Serializer};


/// Internal in-vault staging artifacts are intentionally outside the wire
/// namespace. Never accept this prefix (including case aliases) as a user
/// sync path or remote object name.
pub(crate) fn is_internal_stage_name(name: &str) -> bool {
    const PREFIX: &str = ".akula-sync-stage-";
    name.get(..PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(PREFIX))
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum SyncPathError {
    InvalidPath,
    ReservedName,
    Collision { first: String, second: String },
    Io(String),
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct SyncRelativePath(String);

// Incoming wire paths must pass the same portability checks as local paths.
impl Serialize for SyncRelativePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SyncRelativePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let source = String::deserialize(deserializer)?;
        Self::parse(&source).map_err(|error| serde::de::Error::custom(format!("{error:?}")))
    }
}

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
            // A crash-stage filename cannot be imported from a cloud
            // manifest, indexed as a note or mistaken for a regular attachment.
            if is_internal_stage_name(&normalized) {
                return Err(SyncPathError::ReservedName);
            }
            let base = normalized.split('.').next().unwrap_or("").to_ascii_uppercase();
            // Windows treats ISO-8859-1 superscript 1/2/3 as serial-port
            // digits too: COM¹ and LPT³ are device names, including when
            // followed by an extension. These are multi-byte in UTF-8.
            let serial_device = base
                .strip_prefix("COM")
                .or_else(|| base.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    let mut chars = suffix.chars();
                    matches!(
                        (chars.next(), chars.next()),
                        (Some('1'..='9' | '¹' | '²' | '³'), None)
                    )
                });
            if matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || serial_device
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
    fn rejects_superscript_windows_device_aliases_in_nested_and_wire_paths() {
        // Windows reserves these device aliases in every directory, and
        // NUL.tar.gz is a device alias even with multiple extensions.
        for component in ["COM¹", "COM².md", "COM³.tar.gz",
            "LPT¹", "LPT².txt", "LPT³.backup", "con.md", "NUL.tar.gz"]
        {
            let path = format!("nested/{component}");
            assert_eq!(
                SyncRelativePath::parse(&path),
                Err(SyncPathError::ReservedName),
                "{path:?}"
            );
            let wire = serde_json::to_string(&path).unwrap();
            assert!(
                serde_json::from_str::<SyncRelativePath>(&wire).is_err(),
                "{wire:?}"
            );
        }
        // COM10 is not the legacy COM1 device alias.
        assert_eq!(
            SyncRelativePath::parse("nested/COM10.md").unwrap().as_str(),
            "nested/COM10.md"
        );
    }

    #[test]
    fn a_wire_path_is_validated_and_normalized_on_decode() {
        let canonical: SyncRelativePath =
            serde_json::from_str(r#""notes/caf\u00e9.md""#).unwrap();
        assert_eq!(canonical.as_str(), "notes/caf\u{e9}.md");
        for invalid in [r#""../outside.md""#, r#""CON.md""#, r#""C:/folder.md""#] {
            assert!(serde_json::from_str::<SyncRelativePath>(invalid).is_err());
        }
        assert_eq!(
            serde_json::to_string(&canonical).unwrap(),
            "\"notes/caf\u{e9}.md\""
        );
    }

    #[test]
    fn refuses_reserved_local_create_stage_namespace_on_wire_and_paths() {
        use super::is_internal_stage_name;
        let stage = ".akula-sync-stage-11111111-1111-4111-8111-111111111111";
        assert!(is_internal_stage_name(stage));
        assert!(is_internal_stage_name(".AKULA-SYNC-STAGE-anything"));
        assert!(!is_internal_stage_name(".akula-sync-stages-unrelated"));
        for path in [
            stage.to_string(),
            format!("notes/{stage}"),
            "notes/.AKULA-SYNC-STAGE-notes.md".into(),
            "images/.akula-sync-stage-".into(),
        ] {
            assert_eq!(SyncRelativePath::parse(&path), Err(SyncPathError::ReservedName));
            let json = serde_json::to_string(&path).unwrap();
            assert!(serde_json::from_str::<SyncRelativePath>(&json).is_err());
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
