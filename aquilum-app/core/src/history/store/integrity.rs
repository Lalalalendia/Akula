//! BLAKE3 binding for fixed history versions without a second sidecar commit.
//! The filename (version ID) and contents are published by one atomic rename.
//! This is not a claim of hostile same-user authenticity.

use super::VersionFile;
use crate::files::document::text_from_bytes;
use crate::history::milestone::{Snapshot, Source};
use std::fs;
use std::path::Path;

const SEAL: &str = "~b3-";

fn split_seal(label: &str) -> Option<(&str, Option<&str>)> {
    let Some((source, digest)) = label.rsplit_once(SEAL) else {
        return Some((label, None));
    };
    if digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    Some((source, Some(digest)))
}

fn expected_hash(id: &str) -> Option<Option<&str>> {
    let stem = id.strip_suffix(".md")?;
    let label = stem.splitn(3, '_').nth(2)?;
    split_seal(label).map(|(_, seal)| seal)
}

pub(super) fn version_label(snapshot: &Snapshot<'_>) -> String {
    let label = snapshot.source.file_label();
    // Source::Me may coalesce in-place; its active IDs are never sealed.
    if snapshot.source == Source::Me {
        label
    } else {
        format!("{label}{SEAL}{}", blake3::hash(snapshot.text.as_bytes()).to_hex())
    }
}

pub(super) fn is_sealed_name(id: &str) -> bool {
    id.contains(SEAL)
}

pub(super) fn parse_version(name: String) -> Option<VersionFile> {
    let mut parts = name.strip_suffix(".md")?.splitn(3, '_');
    let at_ms = parts.next()?.parse().ok()?;
    let device = parts.next()?.to_owned();
    let (label, digest) = split_seal(parts.next()?)?;
    let source = Source::from_label(label)?;
    if digest.is_some() && source == Source::Me {
        return None;
    }
    Some(VersionFile {
        id: name,
        at_ms,
        device,
        source,
        from_ms: source.origin_ms(),
        name: None,
        is_current: false,
    })
}

pub(super) fn read_version(dir: &Path, version: &VersionFile) -> Option<String> {
    let seal = expected_hash(&version.id)?;
    let raw = fs::read(dir.join(&version.id)).ok()?;
    if let Some(expected) = seal {
        // Check raw bytes before the editor's existing CRLF normalization.
        if blake3::hash(&raw).to_hex().as_str() != expected {
            return None;
        }
    }
    // Legacy and Source::Me remain readable but have no integrity assertion.
    text_from_bytes(raw).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::store::{update_version_text, versions, write_version};
    use std::borrow::Cow;
    use std::time::{Duration, UNIX_EPOCH};

    fn snapshot<'a>(text: &'a str, source: Source) -> Snapshot<'a> {
        Snapshot {
            text: Cow::Borrowed(text),
            source,
            at: UNIX_EPOCH + Duration::from_secs(170000),
        }
    }

    #[test]
    fn sealed_agent_id_rejects_changed_bytes_and_in_place_updates() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_version(temp.path(), "dev", &snapshot("B 🦈", Source::Agent)).unwrap();
        let versions = versions(temp.path());
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].source, Source::Agent);
        assert!(versions[0].id.contains(SEAL));
        assert_eq!(read_version(temp.path(), &versions[0]).as_deref(), Some("B 🦈"));
        assert!(update_version_text(temp.path(), &versions[0].id, "replace").is_err());
        fs::write(path, b"tampered").unwrap();
        assert!(read_version(temp.path(), &versions[0]).is_none());
    }

    #[test]
    fn mutable_typing_and_old_history_keep_their_previous_semantics() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("1000_dev_agent.md"), b"old\r\ntext").unwrap();
        let path = write_version(temp.path(), "dev", &snapshot("typing", Source::Me)).unwrap();
        let items = versions(temp.path());
        assert_eq!(items.len(), 2);
        assert_eq!(read_version(temp.path(), &items[0]).as_deref(), Some("old\ntext"));
        assert!(!path.to_string_lossy().contains(SEAL));
        update_version_text(temp.path(), &items[1].id, "more typing").unwrap();
        assert_eq!(read_version(temp.path(), &items[1]).as_deref(), Some("more typing"));
    }

    #[test]
    fn sealed_restore_keeps_origin_and_hashes_raw_crlf_before_normalization() {
        let temp = tempfile::tempdir().unwrap();
        let path = write_version(
            temp.path(), "dev", &snapshot("a\r\nb", Source::Restore(42)),
        ).unwrap();
        let v = versions(temp.path()).remove(0);
        assert_eq!(v.from_ms, Some(42));
        assert_eq!(fs::read(&path).unwrap(), b"a\r\nb");
        assert_eq!(read_version(temp.path(), &v).as_deref(), Some("a\nb"));
        fs::write(&path, b"a\nb").unwrap();
        assert!(read_version(temp.path(), &v).is_none());
    }

    #[test]
    fn malformed_marker_is_not_reclassified_as_legacy() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("1_dev_agent~b3-invalid.md"), b"x").unwrap();
        assert!(versions(temp.path()).is_empty());
    }
}
