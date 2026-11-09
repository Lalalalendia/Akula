//! Copy a known source tree into a reserved, fresh directory. This is not a
//! filesystem snapshot: external writers can still race with the final check.
//! On any conflict/error leave the unregistered copy in place for inspection.
use aquilum_core::search::paths::is_hidden_name;
use std::collections::BTreeMap;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_ENTRIES: usize = 200_000;
const MAX_BYTES: u64 = 50 * 1024 * 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_NAME_ATTEMPTS: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryKind {
    File,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fingerprint {
    kind: EntryKind,
    len: u64,
    modified: Option<SystemTime>,
}
type Manifest = BTreeMap<PathBuf, Fingerprint>;

fn linked(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Includes junctions and non-symlink reparse points.
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

fn fingerprint(metadata: &Metadata) -> io::Result<Fingerprint> {
    if linked(metadata) {
        return Err(io::Error::new(ErrorKind::InvalidData, "linked file or folder is not allowed"));
    }
    let (kind, len) = if metadata.is_dir() {
        (EntryKind::Folder, 0)
    } else if metadata.is_file() {
        (EntryKind::File, metadata.len())
    } else {
        return Err(io::Error::new(ErrorKind::InvalidData, "unsupported special file"));
    };
    Ok(Fingerprint { kind, len, modified: metadata.modified().ok() })
}

fn walk(root: &Path, relative: &Path, depth: usize, manifest: &mut Manifest, bytes: &mut u64) -> io::Result<()> {
    if depth > MAX_DEPTH {
        return Err(io::Error::other("knowledge base exceeds folder depth limit"));
    }
    for entry in fs::read_dir(root.join(relative))? {
        let entry = entry?;
        // A duplicated knowledge base is an independent *content* copy, not
        // a clone of history, sync enrollment, Git or another app's state.
        // Apply Akula's existing service-directory policy at every depth.
        // This deliberately omits .aquilum/history and .akula* metadata.
        if is_hidden_name(&entry.file_name()) {
            continue;
        }
        let relative_path = relative.join(entry.file_name());
        let md = fs::symlink_metadata(entry.path())?;
        let info = fingerprint(&md)?;
        if manifest.len() >= MAX_ENTRIES {
            return Err(io::Error::other("knowledge base exceeds entry limit"));
        }
        if info.kind == EntryKind::File {
            *bytes = bytes.checked_add(info.len).ok_or_else(|| io::Error::other("copy size overflow"))?;
            if *bytes > MAX_BYTES {
                return Err(io::Error::other("knowledge base exceeds size limit"));
            }
        }
        let is_folder = info.kind == EntryKind::Folder;
        manifest.insert(relative_path.clone(), info);
        if is_folder {
            walk(root, &relative_path, depth + 1, manifest, bytes)?;
        }
    }
    Ok(())
}

fn manifest(root: &Path) -> io::Result<Manifest> {
    if fingerprint(&fs::symlink_metadata(root)?)?.kind != EntryKind::Folder {
        return Err(io::Error::new(ErrorKind::InvalidInput, "source is not a real directory"));
    }
    let mut entries = Manifest::new();
    let mut total = 0;
    walk(root, Path::new(""), 0, &mut entries, &mut total)?;
    Ok(entries)
}

fn digest_file(path: &Path) -> io::Result<blake3::Hash> {
    if fingerprint(&fs::symlink_metadata(path)?)?.kind != EntryKind::File {
        return Err(io::Error::new(ErrorKind::InvalidData, "source changed type"));
    }
    let mut reader = File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 { break; }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize())
}

fn copy_checked_file(from: &Path, to: &Path, expected: &Fingerprint) -> io::Result<()> {
    if fingerprint(&fs::symlink_metadata(from)?)? != *expected {
        return Err(io::Error::other("source changed before copying"));
    }
    let mut reader = File::open(from)?;
    let mut writer = OpenOptions::new().write(true).create_new(true).open(to)?;
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut copied = 0u64;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 { break; }
        writer.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
        copied += read as u64;
        if copied > expected.len {
            return Err(io::Error::other("source file grew during copying"));
        }
    }
    writer.sync_all()?;
    drop(writer);
    if copied != expected.len || fingerprint(&fs::symlink_metadata(from)?)? != *expected {
        return Err(io::Error::other("source file changed during copying"));
    }
    if digest_file(from)? != hash.finalize() || digest_file(to)? != hash.finalize() {
        return Err(io::Error::other("source or destination content changed while copying"));
    }
    Ok(())
}

pub(super) fn duplicate_folder(source: &Path, suffix: &str) -> Result<PathBuf, String> {
    if suffix.trim().is_empty() {
        return Err("invalid copy suffix".into());
    }
    // Refuse linked roots and pre-existing special entries before creating anything.
    let initial = manifest(source).map_err(|error| error.to_string())?;
    let parent = source.parent().ok_or("source has no parent folder")?;
    let name = source.file_name().ok_or("source has no folder name")?.to_string_lossy();
    if name.is_empty() {
        return Err("invalid source name".into());
    }
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let label = if attempt == 1 { format!("{name} ({suffix})") } else { format!("{name} ({suffix} {attempt})") };
        let destination = parent.join(label);
        match fs::create_dir(&destination) {
            Ok(()) => {
                let result = (|| -> io::Result<()> {
                    for (relative, info) in &initial {
                        let input = source.join(relative);
                        let output = destination.join(relative);
                        if fingerprint(&fs::symlink_metadata(&input)?)? != *info {
                            return Err(io::Error::other("source changed during copying"));
                        }
                        match info.kind {
                            EntryKind::Folder => fs::create_dir(&output)?,
                            EntryKind::File => copy_checked_file(&input, &output, info)?,
                        }
                    }
                    if manifest(source)? != initial {
                        return Err(io::Error::other("source tree changed during copying"));
                    }
                    let dest_manifest = manifest(&destination)?;
                    if dest_manifest.keys().ne(initial.keys()) ||
                        dest_manifest.iter().any(|(name, info)| initial.get(name).is_none_or(|original|
                            info.kind != original.kind || info.len != original.len))
                    {
                        return Err(io::Error::other("destination tree differs from source"));
                    }
                    Ok(())
                })();
                // Never automatically delete incomplete copies: cleanup requires
                // an explicit later user action, independently of this workflow.
                return match result {
                    Ok(()) => Ok(destination),
                    Err(error) => Err(format!("{error}. Incomplete copy retained at {}", destination.display())),
                };
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("no unused copy folder name available".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_rejects_non_file_special_nodes() {
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("Notes");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("note.md"), "One").unwrap();
        let before = manifest(&source).unwrap();
        fs::write(source.join("later.md"), "Two").unwrap();
        assert_ne!(before, manifest(&source).unwrap());
    }

    #[test]
    fn copies_content_without_cloning_history_or_sync_metadata() {
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("Knowledge");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("note.md"), "keep").unwrap();
        for folder in [".aquilum/history", ".akula/credentials", ".quantum/history", ".git", ".obsidian", ".trash", "nested/.aquilum/history"] {
            let secret = source.join(folder);
            fs::create_dir_all(&secret).unwrap();
            fs::write(secret.join("private"), "SECRET").unwrap();
        }
        fs::write(source.join(".akula-secret"), "do not clone").unwrap();
        let copy = duplicate_folder(&source, "copy").unwrap();
        assert_eq!(fs::read_to_string(copy.join("note.md")).unwrap(), "keep");
        for folder in [".aquilum", ".akula", ".quantum", ".git", ".obsidian", ".trash", "nested/.aquilum"] {
            assert!(!copy.join(folder).exists(), "must not clone service state: {folder}");
        }
        assert!(!copy.join(".akula-secret").exists());
        assert!(source.join(".aquilum/history/private").is_file());
        assert!(source.join(".akula/credentials/private").is_file());
    }

    #[test]
    fn rejects_existing_file_at_destination_without_overwriting() {
        let temp = crate::test_dir::tempdir();
        let source = temp.path().join("notes");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a.md"), "GOOD").unwrap();
        let target = temp.path().join("notes (copy)");
        fs::write(&target, "KEEP").unwrap();
        let path = duplicate_folder(&source, "copy").unwrap();
        assert_eq!(path.file_name().unwrap(), "notes (copy 2)");
        assert_eq!(fs::read_to_string(target).unwrap(), "KEEP");
    }
}
