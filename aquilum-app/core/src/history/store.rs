use super::milestone::{Snapshot, Source};
use crate::files::document::{read_text, write_history_snapshot_atomic_impl};
use crate::files::error::FileCommandError;
use crate::files::relocate::{remove_empty_folders, remove_files};
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

mod integrity;
use integrity::parse_version;

pub const AQUILUM_FOLDER: &str = ".aquilum";
const QUANTUM_FOLDER: &str = ".quantum";
const HISTORY_FOLDER: &str = "history";
const VERSION_EXTENSION: &str = ".md";
const NAME_EXTENSION: &str = ".name";
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionFile {
    pub id: String,
    pub at_ms: u64,
    pub device: String,
    pub source: Source,
    pub from_ms: Option<u64>,
    pub name: Option<String>,
    pub is_current: bool,
}

fn history_root(base: &Path) -> PathBuf {
    base.join(AQUILUM_FOLDER).join(HISTORY_FOLDER)
}

/// Strict pathname *preflight* for housekeeping, not an OS-handle-pinned
/// authorization. Never silently follow .quantum/.aquilum/junction aliases.
fn plain_entry(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    #[cfg(windows)]
    let linked = {
        use std::os::windows::fs::MetadataExt;
        const REPARSE_POINT: u32 = 0x0000_0400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let linked = metadata.file_type().is_symlink();
    if linked || (!metadata.is_dir() && !metadata.is_file()) {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("unsafe history alias or file type: {}", path.display()),
        ));
    }
    Ok(())
}

fn plain_directory(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            plain_entry(path, &metadata)?;
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    ErrorKind::NotADirectory,
                    format!("not a history directory: {}", path.display()),
                ));
            }
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Every existing ancestor from the chosen workspace root to a history
/// directory must be a real directory. Missing descendants are okay only
/// for the *destination* of an upcoming migration; callers check the source.
fn checked_history_prefix(base: &Path, directory: &Path) -> io::Result<()> {
    let relative = directory.strip_prefix(base).map_err(|_| {
        io::Error::new(ErrorKind::PermissionDenied, "history path outside the selected base")
    })?;
    if !plain_directory(base)? {
        return Err(io::Error::new(ErrorKind::NotFound, "history workspace root is missing"));
    }
    let mut current = base.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(ErrorKind::PermissionDenied, "invalid history component"));
        };
        current.push(name);
        plain_directory(&current)?;
    }
    Ok(())
}

/// The move-intent directory receives the same no-alias prefix checks as
/// normal version history, before any journal file is opened or created.
pub(super) fn checked_history_recovery_directory(base: &Path) -> io::Result<()> {
    checked_history_prefix(base, &base.join(AQUILUM_FOLDER).join("history-moves"))
}

/// Fail closed on pre-existing symlink/junction aliases before any ordinary
/// HistoryService read, write or relocation. Reuse the migration preflight,
/// but require this path to be inside THIS base's .aquilum/history tree.
/// This is a pathname check, not protection against external TOCTOU swaps.
pub(crate) fn checked_history_note_directory(base: &Path, directory: &Path) -> io::Result<()> {
    let root = history_root(base);
    let relative = directory.strip_prefix(&root).map_err(|_| {
        io::Error::new(ErrorKind::PermissionDenied, "history note is outside its selected root")
    })?;
    if !relative.components().all(|component| matches!(component, Component::Normal(_))) {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "invalid history note path component",
        ));
    }
    checked_history_prefix(base, directory)
}

/// WalkDir's default no-follow setting alone is not enough when the root
/// itself is an alias, on Windows junctions, or if traversal errors get
/// discarded by filter_map(Result::ok). Refuse the entire maintenance pass.
fn collect_plain_history_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(io::Error::other)?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(path)?;
        plain_entry(path, &metadata)?;
        if metadata.is_file() {
            files.push(path.to_path_buf());
        }
    }
    Ok(files)
}

fn history_preflight_error(action: &str, base: &Path, error: &io::Error) {
    eprintln!(
        "[aquilum:history] {action} refused for {}: {error}",
        base.display()
    );
}

pub(super) fn adopt_quantum_history(base: &Path) {
    let legacy = base.join(QUANTUM_FOLDER);
    let legacy_history = legacy.join(HISTORY_FOLDER);
    let target = history_root(base);
    let prepare = || -> io::Result<Option<Vec<PathBuf>>> {
        checked_history_prefix(base, &legacy_history)?;
        if !plain_directory(&legacy_history)? {
            return Ok(None);
        }
        checked_history_prefix(base, &target)?;
        let source_files = collect_plain_history_files(&legacy_history)?;
        // Existing destination aliases must not cause a partial migration,
        // even if the link is in a different note's history subdirectory.
        if plain_directory(&target)? {
            collect_plain_history_files(&target)?;
        }
        for source in &source_files {
            let relative = source.strip_prefix(&legacy_history).map_err(io::Error::other)?;
            let destination = target.join(relative);
            checked_history_prefix(base, source.parent().ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "history source has no parent")
            })?)?;
            checked_history_prefix(base, destination.parent().ok_or_else(|| {
                io::Error::new(ErrorKind::InvalidInput, "history destination has no parent")
            })?)?;
        }
        Ok(Some(source_files))
    };
    let files = match prepare() {
        Ok(Some(files)) => files,
        Ok(None) => return,
        Err(error) => {
            history_preflight_error("migration", base, &error);
            return;
        }
    };
    for file in files {
        let Ok(relative) = file.strip_prefix(&legacy_history) else {
            return;
        };
        let destination = target.join(relative);
        let parent = match destination.parent() {
            Some(parent) => parent,
            None => return,
        };
        // Re-check existing ancestors before each individual rename.
        if let Err(error) = checked_history_prefix(base, parent)
            .and_then(|()| checked_history_prefix(base, file.parent().unwrap_or(&legacy_history)))
            .and_then(|()| {
                let metadata = fs::symlink_metadata(&file)?;
                plain_entry(&file, &metadata)?;
                if !metadata.is_file() {
                    return Err(io::Error::new(ErrorKind::PermissionDenied, "source is not a file"));
                }
                Ok(())
            })
        {
            history_preflight_error("migration", base, &error);
            return;
        }
        // symlink_metadata, not exists(), must detect even dangling links.
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                if let Err(error) = plain_entry(&destination, &metadata) {
                    history_preflight_error("migration", base, &error);
                    return;
                }
                continue; // Existing versions are never overwritten.
            }
            Err(error) if error.kind() == ErrorKind::NotFound => (),
            Err(error) => {
                history_preflight_error("migration", base, &error);
                return;
            }
        };
        let moved = fs::create_dir_all(parent)
            .and_then(|()| checked_history_prefix(base, parent))
            .and_then(|()| fs::rename(&file, &destination));
        if let Err(error) = moved {
            eprintln!("[aquilum:history] версия не перенесена из {}: {error}", file.display());
            return; // Avoid subsequent partial operations on an unsafe tree.
        }
    }
    if let Err(error) = checked_history_prefix(base, &legacy_history) {
        history_preflight_error("migration cleanup", base, &error);
        return;
    }
    remove_empty_folders(&legacy);
    let _ = fs::remove_dir(&legacy);
}

pub fn history_dir(base: &Path, relative: &Path) -> PathBuf {
    history_root(base).join(relative)
}

pub(super) fn write_version(
    dir: &Path,
    device: &str,
    snapshot: &Snapshot<'_>,
) -> Result<PathBuf, FileCommandError> {
    #[cfg(feature = "save-stage-probe")]
    let profile_start = std::time::Instant::now();
    let label = integrity::version_label(snapshot);
    // Distinct source labels can otherwise share a millisecond, causing history
    // ordering to disagree with the order in which versions were written.
    let taken = versions(dir)
        .into_iter()
        .filter(|version| version.device == device)
        .map(|version| version.at_ms)
        .collect::<std::collections::HashSet<_>>();
    #[cfg(feature = "save-stage-probe")]
    let profile_scanned = profile_start.elapsed();
    let first_available = (millis(snapshot.at)..)
        .find(|at| !taken.contains(at))
        .unwrap_or_else(|| millis(snapshot.at));
    let path = free_name(dir, first_available, |at| {
        format!("{at:013}_{device}_{label}{VERSION_EXTENSION}")
    });
    fs::create_dir_all(dir)?;
    #[cfg(feature = "save-stage-probe")]
    let profile_named = profile_start.elapsed();
    write_history_snapshot_atomic_impl(&path, snapshot.text.as_ref())?;
    #[cfg(feature = "save-stage-probe")]
    {
        let profile_end = profile_start.elapsed();
        let ms = |duration: std::time::Duration| duration.as_secs_f64() * 1000.0;
        eprintln!(
            "AKULA_SAVE_VERSION kind={} bytes={} listing_ms={:.4} path_ms={:.4} atomic_ms={:.4} total_ms={:.4}",
            if snapshot.source == Source::Start { "baseline" } else { "current" },
            snapshot.text.len(),
            ms(profile_scanned),
            ms(profile_named - profile_scanned),
            ms(profile_end - profile_named),
            ms(profile_end),
        );
    }
    Ok(path)
}

pub fn versions(dir: &Path) -> Vec<VersionFile> {
    let mut versions = files(dir)
        .filter_map(|(file_name, _)| parse_version(file_name))
        .collect::<Vec<_>>();
    versions.sort_by(|left, right| {
        (
            left.at_ms,
            &left.device,
            left.source != Source::Start,
            &left.id,
        )
            .cmp(&(
                right.at_ms,
                &right.device,
                right.source != Source::Start,
                &right.id,
            ))
    });
    versions
}

pub fn read_version(dir: &Path, version: &VersionFile) -> Option<String> {
    integrity::read_version(dir, version)
}

pub fn has_text(dir: &Path, version: &VersionFile, text: &str) -> bool {
    let range = text.len() as u64..=text.len().saturating_add(text.matches('\n').count()) as u64;
    fs::metadata(dir.join(&version.id)).is_ok_and(|m| range.contains(&m.len()))
        && read_version(dir, version).is_some_and(|stored| stored == text)
}

#[cfg(test)]
pub fn latest_text(dir: &Path) -> Option<String> {
    versions(dir).last().and_then(|version| read_version(dir, version))
}

pub fn version_at(id: &str) -> Option<u64> {
    parse_version(id.to_owned()).map(|version| version.at_ms)
}

pub(super) fn update_version_text(
    dir: &Path,
    version_id: &str,
    text: &str,
) -> Result<(), FileCommandError> {
    if integrity::is_sealed_name(version_id) {
        return Err(FileCommandError::Io {
            message: "sealed history version may not be overwritten".to_owned(),
        });
    }
    write_history_snapshot_atomic_impl(&dir.join(version_id), text)
}

pub(super) fn remove_version_and_names(
    dir: &Path,
    version_id: &str,
) -> Result<(), FileCommandError> {
    // Enumerate labels before the destructive phase. An unreadable directory
    // must not result in an acknowledged, incomplete removal.
    let mut labels = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name();
        if let Some((version, _, _)) = name_mark(&file_name.to_string_lossy()) {
            if version == version_id {
                labels.push(entry.path());
            }
        }
    }
    fs::remove_file(dir.join(version_id))?;
    for label in labels {
        fs::remove_file(label)?;
    }
    Ok(())
}

pub fn system_time_from_ms(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

pub(super) fn write_name(
    dir: &Path,
    device: &str,
    version_id: &str,
    name: &str,
    at: SystemTime,
) -> Result<(), FileCommandError> {
    let path = free_name(dir, millis(at), |at| {
        format!("{version_id}.{at:013}_{device}{NAME_EXTENSION}")
    });
    write_history_snapshot_atomic_impl(&path, name.trim())
}

pub fn names(dir: &Path) -> HashMap<String, String> {
    let mut marks = files(dir)
        .filter_map(|(file_name, path)| {
            let (version, at, device) = name_mark(&file_name)?;
            Some((version.to_owned(), at, device.to_owned(), path))
        })
        .collect::<Vec<_>>();
    marks.sort();
    marks
        .into_iter()
        .map(|(version, _, _, path)| (version, path))
        .collect::<HashMap<_, _>>()
        .into_iter()
        .filter_map(|(version, path)| {
            let name = read_text(&path).ok()?.trim().to_owned();
            (!name.is_empty()).then_some((version, name))
        })
        .collect()
}

pub(super) fn remove_expired(vault: &Path, retention_days: u32) -> u64 {
    let root = history_root(vault);
    if retention_days == 0 {
        return 0;
    }
    let files = match checked_history_prefix(vault, &root).and_then(|()| {
        if !plain_directory(&root)? {
            return Ok(None);
        }
        collect_plain_history_files(&root).map(Some)
    }) {
        Ok(Some(files)) => files,
        Ok(None) => return 0,
        Err(error) => {
            history_preflight_error("retention", vault, &error);
            return 0;
        }
    };
    let cutoff = millis(SystemTime::now()).saturating_sub(millis(UNIX_EPOCH + DAY * retention_days));
    let mut named = HashMap::<PathBuf, HashMap<String, String>>::new();
    let expired = files
        .into_iter()
        .filter_map(|file| {
            let version = parse_version(file.file_name()?.to_string_lossy().into_owned())?;
            let labels = named
                .entry(file.parent()?.to_path_buf())
                .or_insert_with_key(|dir| names(dir));
            (version.at_ms < cutoff && !labels.contains_key(&version.id)).then_some(file)
        })
        .collect::<Vec<_>>();
    // Revalidate before the destructive phase; this remains a pathname
    // preflight and does NOT close races against independent OS processes.
    if let Err(error) = checked_history_prefix(vault, &root)
        .and_then(|()| collect_plain_history_files(&root).map(|_| ()))
    {
        history_preflight_error("retention delete", vault, &error);
        return 0;
    }
    let removed = remove_files(&expired);
    let orphans = named.keys().flat_map(|dir| orphan_names(dir)).collect::<Vec<_>>();
    remove_files(&orphans);
    remove_empty_folders(&root);
    removed
}

fn orphan_names(dir: &Path) -> Vec<PathBuf> {
    files(dir)
        .filter_map(|(file_name, path)| {
            let (version, _, _) = name_mark(&file_name)?;
            (!dir.join(version).exists()).then_some(path)
        })
        .collect()
}

fn name_mark(file_name: &str) -> Option<(&str, u64, &str)> {
    let (version, stamp) = file_name.strip_suffix(NAME_EXTENSION)?.rsplit_once('.')?;
    let (at, device) = stamp.split_once('_')?;
    Some((version, at.parse().ok()?, device))
}

fn files(dir: &Path) -> impl Iterator<Item = (String, PathBuf)> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
}

fn millis(at: SystemTime) -> u64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

fn free_name(dir: &Path, at_ms: u64, name: impl Fn(u64) -> String) -> PathBuf {
    (at_ms..)
        .map(|at| dir.join(name(at)))
        .find(|candidate| !candidate.exists())
        .unwrap_or_else(|| dir.join(name(at_ms)))
}

#[cfg(test)]
mod tests {
    use super::{
        adopt_quantum_history, history_dir, history_root, latest_text, names, remove_expired,
        versions, write_name, write_version, DAY,
    };
    use crate::history::milestone::{Snapshot, Source};
    use std::fs;
    use std::path::Path;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn snapshot(text: &str, source: Source, seconds: u64) -> Snapshot<'static> {
        Snapshot {
            text: text.to_owned().into(),
            source,
            at: UNIX_EPOCH + Duration::from_secs(seconds),
        }
    }

    #[cfg(unix)]
    #[test]
    fn retention_refuses_linked_aquilum_root_and_nested_history_aliases() {
        use std::os::unix::fs::symlink;
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let history = outside.path().join("history");
        let outside_note = history.join("Приватная.md");
        fs::create_dir_all(&outside_note).unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let version = write_version(
            &outside_note, "test", &snapshot("must survive", Source::Me, now - 40 * DAY.as_secs())
        ).unwrap();
        symlink(outside.path(), vault.path().join(".aquilum")).unwrap();
        assert_eq!(remove_expired(vault.path(), 30), 0);
        assert!(version.is_file());
        fs::remove_file(vault.path().join(".aquilum")).unwrap();

        let inner = vault.path().join(".aquilum");
        fs::create_dir(&inner).unwrap();
        symlink(&history, inner.join("history")).unwrap();
        assert_eq!(remove_expired(vault.path(), 30), 0);
        assert!(version.is_file());
        fs::remove_file(inner.join("history")).unwrap();

        let genuine = history_root(vault.path());
        fs::create_dir_all(&genuine).unwrap();
        let good = genuine.join("Нормальная.md");
        fs::create_dir(&good).unwrap();
        let good_version = write_version(
            &good, "test", &snapshot("should remain on failed scan", Source::Me, now - 40 * DAY.as_secs())
        ).unwrap();
        symlink(&outside_note, genuine.join("Ссылка.md")).unwrap();
        assert_eq!(remove_expired(vault.path(), 30), 0);
        assert!(version.is_file());
        assert!(good_version.is_file(), "no partial retention on unsafe tree");
    }

    #[cfg(unix)]
    #[test]
    fn migration_refuses_legacy_and_destination_aliases_without_moving_bytes() {
        use std::os::unix::fs::symlink;
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let source_dir = outside.path().join("history").join("Имя.md");
        fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("1_test_me.md");
        fs::write(&source, "original content").unwrap();

        symlink(outside.path(), vault.path().join(".quantum")).unwrap();
        adopt_quantum_history(vault.path());
        assert!(source.exists());
        assert!(!vault.path().join(".aquilum").exists());
        fs::remove_file(vault.path().join(".quantum")).unwrap();

        let legacy_dir = vault.path().join(".quantum/history/Имя.md");
        fs::create_dir_all(&legacy_dir).unwrap();
        let local_source = legacy_dir.join("1_test_me.md");
        fs::write(&local_source, "local original").unwrap();
        symlink(outside.path(), vault.path().join(".aquilum")).unwrap();
        adopt_quantum_history(vault.path());
        assert_eq!(fs::read_to_string(&local_source).unwrap(), "local original");
        assert_eq!(fs::read_to_string(&source).unwrap(), "original content");
        fs::remove_file(vault.path().join(".aquilum")).unwrap();

        let target_root = history_root(vault.path());
        fs::create_dir_all(&target_root).unwrap();
        symlink(&source_dir, target_root.join("Имя.md")).unwrap();
        adopt_quantum_history(vault.path());
        assert!(local_source.exists(), "migration must abort at nested target aliases");
        assert_eq!(fs::read_to_string(&source).unwrap(), "original content");
    }

    #[cfg(windows)]
    #[test]
    fn windows_retention_and_migration_refuse_ntfs_junction_roots() {
        use std::process::Command;
        fn junction(link: &Path, destination: &Path) {
            let output = Command::new("cmd")
                .arg("/C").arg("mklink").arg("/J").arg(link).arg(destination)
                .output().unwrap();
            assert!(output.status.success(), "mklink /J failed: {}",
                String::from_utf8_lossy(&output.stderr));
        }

        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let foreign_dir = outside.path().join("history").join("Idea.md");
        fs::create_dir_all(&foreign_dir).unwrap();
        let foreign_file = write_version(
            &foreign_dir, "win", &snapshot("external", Source::Me, now - 40 * DAY.as_secs())
        ).unwrap();

        junction(&vault.path().join(".aquilum"), outside.path());
        assert_eq!(remove_expired(vault.path(), 30), 0);
        assert!(foreign_file.exists());
        fs::remove_dir(vault.path().join(".aquilum")).unwrap();

        junction(&vault.path().join(".quantum"), outside.path());
        adopt_quantum_history(vault.path());
        assert!(foreign_file.exists());
        assert!(!vault.path().join(".aquilum").exists());
        fs::remove_dir(vault.path().join(".quantum")).unwrap();

        let target_history = history_root(vault.path());
        fs::create_dir_all(&target_history).unwrap();
        let internal_legacy = vault.path().join(".quantum").join("history").join("Idea.md");
        fs::create_dir_all(&internal_legacy).unwrap();
        let internal_file = internal_legacy.join("1_win_me.md");
        fs::write(&internal_file, "local").unwrap();
        junction(&target_history.join("Idea.md"), &foreign_dir);
        adopt_quantum_history(vault.path());
        assert!(internal_file.exists(), "junctions in destination must block migration");
        assert_eq!(fs::read_to_string(&foreign_file).unwrap(), "external");
        fs::remove_dir(target_history.join("Idea.md")).unwrap();
    }

    #[test]
    fn history_kept_under_the_quantum_name_moves_to_aquilum_without_overwriting() {
        let vault = tempfile::tempdir().unwrap();
        let legacy = vault.path().join(".quantum/history/Идея.md");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("1_dev_me.md"), "старая").unwrap();
        fs::write(legacy.join("2_dev_me.md"), "из quantum").unwrap();
        let current = history_dir(vault.path(), Path::new("Идея.md"));
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("2_dev_me.md"), "уже в aquilum").unwrap();

        adopt_quantum_history(vault.path());

        assert_eq!(fs::read_to_string(current.join("1_dev_me.md")).unwrap(), "старая");
        assert_eq!(fs::read_to_string(current.join("2_dev_me.md")).unwrap(), "уже в aquilum");
        assert!(legacy.join("2_dev_me.md").exists());
        assert!(!legacy.join("1_dev_me.md").exists());
    }

    #[test]
    fn the_history_folder_repeats_the_note_path() {
        let vault = Path::new("C:/База");
        assert_eq!(
            history_dir(vault, Path::new("Проекты/Идея.md")),
            vault.join(".aquilum").join("history").join("Проекты/Идея.md")
        );
    }

    #[test]
    fn versions_are_separate_files_ordered_by_time_and_never_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let dir = history_dir(directory.path(), Path::new("Идея.md"));
        write_version(&dir, "aaaa", &snapshot("второй", Source::Agent, 20)).unwrap();
        write_version(&dir, "aaaa", &snapshot("первый", Source::Me, 10)).unwrap();
        let clash = write_version(&dir, "aaaa", &snapshot("тот же миг", Source::Agent, 20)).unwrap();

        let found = versions(&dir);

        assert_eq!(found.iter().map(|version| version.source).collect::<Vec<_>>(), [Source::Me, Source::Agent, Source::Agent]);
        assert_eq!(dir.join(&found[2].id), clash);
        assert_eq!(found[2].at_ms, 20_001, "совпавшее имя сдвигает миллисекунду, а не перезаписывает файл");
        assert_eq!(latest_text(&dir).as_deref(), Some("тот же миг"));
    }

    #[test]
    fn different_sources_from_one_device_in_one_millisecond_keep_write_order() {
        let directory = tempfile::tempdir().unwrap();
        let dir = history_dir(directory.path(), Path::new("same-millisecond.md"));
        let at = UNIX_EPOCH + Duration::from_millis(5);
        let make = |text: &str, source| Snapshot { text: text.to_owned().into(), source, at };
        let first = write_version(&dir, "dev", &make("first", Source::Agent)).unwrap();
        let second = write_version(&dir, "dev", &make("second", Source::Me)).unwrap();
        let found = versions(&dir);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].id, first.file_name().unwrap().to_string_lossy());
        assert_eq!(found[1].id, second.file_name().unwrap().to_string_lossy());
        assert_eq!((found[0].at_ms, found[1].at_ms), (5, 6));
        assert_eq!(latest_text(&dir).as_deref(), Some("second"));
    }

    #[test]
    fn same_millisecond_start_precedes_edits_without_changing_device_order() {
        let directory = tempfile::tempdir().unwrap();
        let dir = history_dir(directory.path(), Path::new("Идея.md"));
        let at = UNIX_EPOCH + Duration::from_millis(20_000);
        let snapshot = |text: &str, source| Snapshot {
            text: text.to_owned().into(),
            source,
            at,
        };
        write_version(&dir, "bbbb", &snapshot("старт", Source::Start)).unwrap();
        write_version(&dir, "bbbb", &snapshot("правка", Source::Me)).unwrap();
        write_version(&dir, "aaaa", &snapshot("другое устройство", Source::Agent)).unwrap();

        let found = versions(&dir);

        assert_eq!(
            found.iter().map(|version| version.source).collect::<Vec<_>>(),
            [Source::Agent, Source::Start, Source::Me]
        );
        assert_eq!(latest_text(&dir).as_deref(), Some("правка"));
    }

    #[test]
    fn a_restored_version_keeps_the_time_of_its_original() {
        let directory = tempfile::tempdir().unwrap();
        let dir = history_dir(directory.path(), Path::new("Идея.md"));
        let written = write_version(&dir, "aaaa", &snapshot("старое", Source::Restore(10_000), 20)).unwrap();

        let found = versions(&dir);

        let digest = blake3::hash("старое".as_bytes()).to_hex();
        assert_eq!(
            written.file_name().unwrap().to_str().unwrap(),
            format!("{:013}_aaaa_restore-0000000010000~b3-{digest}.md", 20_000)
        );
        assert_eq!((found[0].source, found[0].from_ms), (Source::Restore(10_000), Some(10_000)));
    }

    #[test]
    fn the_latest_name_of_a_version_wins_and_an_empty_one_removes_it() {
        let directory = tempfile::tempdir().unwrap();
        let dir = history_dir(directory.path(), Path::new("Идея.md"));
        let id = write_version(&dir, "aaaa", &snapshot("текст", Source::Me, 10)).unwrap();
        let id = id.file_name().unwrap().to_string_lossy().into_owned();
        let at = |seconds| UNIX_EPOCH + Duration::from_secs(seconds);

        write_name(&dir, "aaaa", &id, " Черновик ", at(20)).unwrap();
        write_name(&dir, "bbbb", &id, "До рефакторинга", at(30)).unwrap();
        assert_eq!(names(&dir).get(&id).map(String::as_str), Some("До рефакторинга"));
        assert!(versions(&dir).len() == 1, "файл названия — не версия");

        write_name(&dir, "aaaa", &id, "", at(40)).unwrap();
        assert!(names(&dir).is_empty());
    }

    #[test]
    fn retention_keeps_named_versions_and_drops_names_of_removed_ones() {
        let directory = tempfile::tempdir().unwrap();
        let dir = history_dir(directory.path(), Path::new("Идея.md"));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let old = now - 40 * DAY.as_secs();
        let named = write_version(&dir, "aaaa", &snapshot("важная", Source::Me, old)).unwrap();
        let plain = write_version(&dir, "aaaa", &snapshot("обычная", Source::Me, old + 1)).unwrap();
        let id = |path: &Path| path.file_name().unwrap().to_string_lossy().into_owned();
        write_name(&dir, "aaaa", &id(&named), "Важная", UNIX_EPOCH + Duration::from_secs(old)).unwrap();
        write_name(&dir, "aaaa", &id(&plain), "", UNIX_EPOCH + Duration::from_secs(old)).unwrap();

        assert_eq!(remove_expired(directory.path(), 30), 1);

        assert!(named.exists() && !plain.exists());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2, "осталась версия и её название");
    }

    #[test]
    fn two_devices_never_write_the_same_file() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path().join("Идея.md");
        let first = write_version(&dir, "aaaa", &snapshot("с первого", Source::Me, 10)).unwrap();
        let second = write_version(&dir, "bbbb", &snapshot("со второго", Source::Me, 10)).unwrap();
        assert_ne!(first, second);
        assert_eq!(versions(&dir).len(), 2);
    }

    #[test]
    fn retention_removes_versions_older_than_the_limit_by_their_own_time() {
        let directory = tempfile::tempdir().unwrap();
        let old_note = history_dir(directory.path(), Path::new("Архив/Старая.md"));
        let fresh_note = history_dir(directory.path(), Path::new("Свежая.md"));
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        write_version(&old_note, "aaaa", &snapshot("давно", Source::Me, now - 40 * DAY.as_secs())).unwrap();
        write_version(&fresh_note, "aaaa", &snapshot("давно", Source::Me, now - 40 * DAY.as_secs())).unwrap();
        write_version(&fresh_note, "aaaa", &snapshot("вчера", Source::Me, now - DAY.as_secs())).unwrap();
        fs::write(fresh_note.join("заметки.txt"), "чужой файл").unwrap();

        assert_eq!(remove_expired(directory.path(), 0), 0, "ноль — хранить всегда");
        assert_eq!(remove_expired(directory.path(), 30), 2);

        assert!(!history_root(directory.path()).join("Архив").exists(), "опустевшие папки убраны");
        assert_eq!(latest_text(&fresh_note).as_deref(), Some("вчера"));
        assert!(fresh_note.join("заметки.txt").exists(), "трогаются только файлы версий");
    }
}
