use super::{announce_history, paths_changed, record, report_record};
use crate::app_core::Core;
use crate::files::document::write_file_atomic_impl;
use crate::files::entries::{copy_file_impl, create_binary_file_impl, create_file_impl};
use crate::files::error::FileCommandError;
use crate::files::models::FileWriteResult;
use crate::history::{NoteWrite, Source};
use crate::search::paths::{is_markdown, is_note};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn write(
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: Option<&str>,
    source: Source,
    version_name: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    // Explicit installer-host Core: ONLY write/create have a scoped shared
    // unenrolled epoch in this stage. All other ordinary mutation entrypoints
    // remain HOLD. This must precede guarded_editor::maybe_write: host-mode
    // cannot silently use that independent per-appdata custody route.
    #[cfg(feature = "host-custody")]
    if core.requires_installed_host_epoch() {
        let (result, history_changed) = {
            let _publication = core.ordinary_note_publication_lease()?;
            core.with_unenrolled_host_epoch(path, |permit| {
                let mut file_gate_guard = core.file_mutation_guard_for_host_write(permit)?;
                let before = core.history.disk_before(path, expected_hash);
                let result = write_file_atomic_impl(path, content, expected_hash)?;
                file_gate_guard.release_process_lock();
                #[cfg(test)]
                pause_file_gate_after_primary_before_history();
                let write = NoteWrite {
                    disk_before: before, text: content, hash: &result.hash,
                    source, name: version_name,
                };
                let changed = core.history.try_saved_with_host_epoch(
                    permit, &|| core.known_roots(), path, write,
                ).map_err(|error| FileCommandError::NeedsReview {
                    message: format!(
                        "host-bound primary save may have committed but history failed: {error}; review before retry"
                    ),
                })?;
                Ok((result, changed))
            })?
        };
        // Callbacks and search are deliberately AFTER SQLite COMMIT.
        report_record(core, path, Ok(history_changed));
        if is_markdown(path) {
            core.search.notify_content_path(path.to_path_buf());
        }
        return Ok(result);
    }

    // Opt-in S3-registered editor mode reuses a held external OS lock across
    // primary bytes AND HistoryService. It must run before the ordinary
    // process-global File Gate mutex to preserve the lock order.
    #[cfg(feature = "sync-s3")]
    if let Some(guarded) = crate::sync::guarded_editor::maybe_write(
        core, path, content, expected_hash, source, version_name,
    ) {
        return guarded;
    }
    let (result, history_outcome) = {
        #[cfg(feature = "save-stage-probe")]
        let profile_start = std::time::Instant::now();
        // This lease spans BOTH the primary file replacement and the history
        // attempt. Activation waits for it before flipping durable HOLD.
        // Only enabled for opt-in S3 builds; the unarmed route stays cheap.
        #[cfg(feature = "host-custody")]
        let _publication = core.ordinary_note_publication_lease()?;
        let mut file_gate_guard = core.file_mutation_guard_for_write()?;
        let before = core.history.disk_before(path, expected_hash);
        let result = write_file_atomic_impl(path, content, expected_hash)?;
        file_gate_guard.release_process_lock();
        #[cfg(test)]
        pause_file_gate_after_primary_before_history();
        #[cfg(feature = "save-stage-probe")]
        let profile_primary = profile_start.elapsed();
        let write = NoteWrite {
            disk_before: before, text: content, hash: &result.hash, source,
            name: version_name,
        };
        let outcome = core.history.try_saved(&|| core.known_roots(), path, write);
        #[cfg(feature = "save-stage-probe")]
        {
            let profile_done = profile_start.elapsed();
            eprintln!(
                "AKULA_SAVE_GATE primary_ms={:.4} history_ms={:.4} total_ms={:.4} history_ok={}",
                profile_primary.as_secs_f64() * 1000.0,
                (profile_done - profile_primary).as_secs_f64() * 1000.0,
                profile_done.as_secs_f64() * 1000.0,
                u8::from(outcome.is_ok()),
            );
        }
        (result, outcome)
    };
    // Do NOT invoke any host callback while holding the publication epoch.
    report_record(core, path, history_outcome);
    if is_markdown(path) {
        core.search.notify_content_path(path.to_path_buf());
    }
    Ok(result)
}

/// A narrow internal File Gate variant for an already-held external custody
/// OS lock. It does NOT acquire that lock itself and must never be called
/// outside a verified cooperative writer lifetime. Unlike the legacy editor
/// route, a failed HistoryService write is returned explicitly. The primary
/// bytes might already have changed on ANY failure after writer entry.
#[derive(Debug)]
pub(crate) enum CheckedHistoryError {
    Primary(FileCommandError),
    History(crate::history::service::HistorySaveFailure),
}

/// Backward-compatible checked writer: the extra prepublication identity
/// check is opt-in. Normal local history publication still has one writer.
#[allow(dead_code)]
pub(crate) fn write_checked_history(
    permit: &crate::sync::guarded_note_write::HeldCustodyHistoryPermit<'_>,
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
) -> Result<(FileWriteResult, bool), CheckedHistoryError> {
    write_checked_history_with_prewrite(
        permit, core, path, content, expected_hash, source, version_name,
        || Ok(()),
    )
}

/// P0g-3a: revalidate the prepared PHYSICAL original within Core File Gate,
/// immediately before the sole existing AtomicWriteFile publisher.
///
/// The callback is synchronous trusted host code. It must not reenter File
/// Gate, custody, HistoryService or the network. This path grants no S3 ACK.
#[allow(dead_code)]
pub(crate) fn write_checked_history_with_prewrite(
    permit: &crate::sync::guarded_note_write::HeldCustodyHistoryPermit<'_>,
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
    prewrite: impl FnOnce() -> Result<(), FileCommandError>,
) -> Result<(FileWriteResult, bool), CheckedHistoryError> {
    let (result, evidence) = write_checked_history_under_gate(
        permit, core, path, content, expected_hash, source, version_name,
        HistoryEvidenceMode::Legacy, prewrite,
    )?;
    let CheckedHistoryEvidence::LegacyChanged(changed) = evidence else {
        unreachable!("legacy File Gate cannot return a strict witness");
    };
    Ok((result, changed))
}

/// P0g-3d-03: STRICT, opt-in point-in-time HistoryService witness under the
/// same held external OS custody and the same single File Gate publication.
/// A real external CreateNew version is required; mutable/coalesced, quiet,
/// label-only or no-op history states cannot produce a strict witness.
/// This observation alone is NEVER a durable primary/history/SQLite receipt.
#[allow(dead_code)]
pub(crate) fn write_checked_history_with_prewrite_verified(
    permit: &crate::sync::guarded_note_write::HeldCustodyHistoryPermit<'_>,
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
    prewrite: impl FnOnce() -> Result<(), FileCommandError>,
) -> Result<(FileWriteResult, crate::history::service::VerifiedHistoryVersion), CheckedHistoryError> {
    let (result, evidence) = write_checked_history_under_gate(
        permit, core, path, content, expected_hash, source, version_name,
        HistoryEvidenceMode::ImmutableExternal, prewrite,
    )?;
    let CheckedHistoryEvidence::Verified(version) = evidence else {
        unreachable!("strict File Gate cannot return a legacy bool");
    };
    Ok((result, version))
}

#[derive(Clone, Copy)]
enum HistoryEvidenceMode {
    Legacy,
    ImmutableExternal,
}

/// Typed to prevent accidentally treating a legacy changed/no-change bool
/// as immutable HistoryService evidence.
enum CheckedHistoryEvidence {
    LegacyChanged(bool),
    Verified(crate::history::service::VerifiedHistoryVersion),
}

/// Test-only process-abort probe after the *existing* File Gate publisher.
/// It is absent from all production Core builds. The exact note must live
/// under the disposable parent workdir of a dedicated libtest OS child.
/// Never expose a crash trigger in an installed application or network API.
#[cfg(test)]
fn p0g3d08c_abort_only_after_primary_published(path: &Path) {
    if !matches!(
        std::env::var("AKULA_P0G3D08C_STAGE").as_deref(),
        Ok("after-primary")
    ) {
        return;
    }
    let workdir = std::env::var_os("AKULA_P0G3D08C_WORKDIR")
        .expect("crash test requires a disposable workdir");
    let workdir = Path::new(&workdir);
    // Windows can spell one physical path using different drive casing or
    // \\?\\ namespace prefixes. std::Path::starts_with compares those raw
    // components; use Akula's component-boundary, Windows-aware path check
    // over resolved paths instead. Never use a textual string prefix.
    let canonical_workdir = crate::search::paths::canonical_path(workdir);
    let canonical_target = crate::search::paths::canonical_path(path);
    let relative = crate::search::paths::strip_root(
        &canonical_workdir, &canonical_target,
    ).expect("never abort an unrelated workspace");
    assert_eq!(relative.components().count(), 3,
        "crash target must be one disposable fixture/vault/note");
    assert!(relative.components().next().is_some_and(|part|
        part.as_os_str().to_string_lossy().starts_with("writer-crash-fixture-")),
        "crash target is not inside a named disposable writer fixture");
    assert_eq!(path.file_name(), Some(std::ffi::OsStr::new("Reserved.md")));
    std::fs::write(workdir.join("p0g3d08c-crash-marker"), b"after-primary")
        .expect("persist test-only crash marker before abort");
    std::process::abort();
}

/// The *only* common File Gate primary publisher for both historical and
/// opt-in strict history paths. Do not add another rename/swap authority.
fn write_checked_history_under_gate(
    permit: &crate::sync::guarded_note_write::HeldCustodyHistoryPermit<'_>,
    core: &Core,
    path: &Path,
    content: &str,
    expected_hash: &str,
    source: Source,
    version_name: Option<&str>,
    mode: HistoryEvidenceMode,
    prewrite: impl FnOnce() -> Result<(), FileCommandError>,
) -> Result<(FileWriteResult, CheckedHistoryEvidence), CheckedHistoryError> {
    // The lexical permit is Core and *note path* specific: deny before any
    // preparation callback, File Gate lock or primary mutation.
    if !permit.binds_core_and_note(core, path) {
        return Err(CheckedHistoryError::Primary(FileCommandError::Task {
            message: "held custody permit belongs to a different Core or note".into(),
        }));
    }
    // A strict receipt candidate must be External and bounded BEFORE the
    // destructive primary publication. HistoryService validates immutable
    // CreateNew semantics and exact byte readback separately after the write.
    if matches!(mode, HistoryEvidenceMode::ImmutableExternal)
        && (source != Source::External || content.len() > 32 * 1024 * 1024)
    {
        return Err(CheckedHistoryError::Primary(FileCommandError::Task {
            message: "strict history requires bounded external content".into(),
        }));
    }

    // Keep lock order: held external OS lock -> Core File Gate interlock ->
    // release the process-local mutex -> HistoryService state and filesystem locks.
    let mut file_gate_guard = core.file_mutation_guard_for_custody_write()
        .map_err(CheckedHistoryError::Primary)?;
    prewrite().map_err(CheckedHistoryError::Primary)?;
    let before = core.history.disk_before(path, Some(expected_hash));
    let result = write_file_atomic_impl(path, content, Some(expected_hash))
        .map_err(CheckedHistoryError::Primary)?;
    #[cfg(test)]
    p0g3d08c_abort_only_after_primary_published(path);
    file_gate_guard.release_process_lock();
    let write = NoteWrite {
        disk_before: before, text: content, hash: &result.hash,
        source, name: version_name,
    };
    let evidence = match mode {
        HistoryEvidenceMode::Legacy => {
            CheckedHistoryEvidence::LegacyChanged(
                core.history.try_saved_with_custody(
                    permit, &|| core.known_roots(), path, write,
                ).map_err(CheckedHistoryError::History)?
            )
        }
        HistoryEvidenceMode::ImmutableExternal => {
            CheckedHistoryEvidence::Verified(
                core.history.try_saved_with_custody_verified_version(
                    permit, &|| core.known_roots(), path, write,
                ).map_err(CheckedHistoryError::History)?
            )
        }
    };
    // Only a point-in-time history observation. No native receipt, no
    // baseline commit, and no filesystem power-loss durability claim.
    Ok((result, evidence))
}

#[cfg(test)]
fn pause_file_gate_after_primary_before_history() {
    const MARKER_ENV: &str = "AKULA_FILEGATE_TEST_PAUSE_MARKER";
    const RELEASE_ENV: &str = "AKULA_FILEGATE_TEST_PAUSE_RELEASE";

    let Some(marker) = std::env::var_os(MARKER_ENV) else {
        return;
    };
    let release =
        std::env::var_os(RELEASE_ENV).expect("File Gate test pause marker requires a release path");
    let marker = PathBuf::from(marker);
    let release = PathBuf::from(release);
    std::fs::write(&marker, b"after-primary").expect("write File Gate test pause marker");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !release.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for File Gate test pause release"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub fn create(
    core: &Core,
    path: &Path,
    content: &str,
    source: Source,
    version_name: Option<&str>,
) -> Result<FileWriteResult, FileCommandError> {
    #[cfg(feature = "host-custody")]
    if core.requires_installed_host_epoch() {
        let (result, changed) = {
            let _publication = core.ordinary_note_publication_lease()?;
            core.with_unenrolled_host_epoch(path, |permit| {
                let mut file_gate_guard = core.file_mutation_guard_for_host_write(permit)?;
                let result = create_file_impl(path, content)?;
                file_gate_guard.release_process_lock();
                let history = if is_note(path) {
                    let write = NoteWrite {
                        disk_before: Some((String::new(), SystemTime::now())),
                        text: content, hash: &result.hash, source, name: version_name,
                    };
                    Some(core.history.try_saved_with_host_epoch(
                        permit, &|| core.known_roots(), path, write,
                    ).map_err(|error| FileCommandError::NeedsReview {
                        message: format!(
                            "host-bound create may have committed but history failed: {error}; review before retry"
                        ),
                    })?)
                } else {
                    None
                };
                Ok((result, history))
            })?
        };
        if let Some(changed) = changed {
            report_record(core, path, Ok(changed));
        }
        paths_changed(core, vec![path.to_path_buf()]);
        return Ok(result);
    }
    let (result, history_outcome) = {
        #[cfg(feature = "host-custody")]
        let _publication = core.ordinary_note_publication_lease()?;
        let mut file_gate_guard = core.file_mutation_guard_for_write()?;
        let result = create_file_impl(path, content)?;
        file_gate_guard.release_process_lock();
        let history = if is_note(path) {
            let write = NoteWrite {
                disk_before: Some((String::new(), SystemTime::now())),
                text: content, hash: &result.hash, source, name: version_name,
            };
            Some(core.history.try_saved(&|| core.known_roots(), path, write))
        } else {
            None
        };
        (result, history)
    };
    if let Some(history) = history_outcome {
        report_record(core, path, history);
    }
    paths_changed(core, vec![path.to_path_buf()]);
    Ok(result)
}

pub fn create_binary(
    core: &Core,
    path: &Path,
    bytes: &[u8],
) -> Result<FileWriteResult, FileCommandError> {
    let result = {
        let _guard = core.file_mutation_guard_for_write()?;
        create_binary_file_impl(path, bytes)?
    };
    paths_changed(core, vec![path.to_path_buf()]);
    Ok(result)
}

pub fn copy(core: &Core, from: &Path, to: &Path) -> Result<(), FileCommandError> {
    {
        let _guard = core.file_mutation_guard_for_write()?;
        copy_file_impl(from, to)?;
    }
    paths_changed(core, vec![to.to_path_buf()]);
    Ok(())
}

pub fn name_version(
    core: &Core,
    path: &Path,
    version: Option<&str>,
    name: &str,
) -> Option<String> {
    // Naming a version mutates durable .aquilum history metadata. The
    // cooperative writer authorizes only an existing-note CAS save, not
    // arbitrary history edits. A cold Core with enrolled custody must deny
    // this entrypoint as well; its Option API represents refusal as None.
    // This does not fence direct HistoryService users or concurrent activation.
    if core.documents.requires_guarded_participant_hold() {
        return None;
    }
    let named = core.history.name(&|| core.known_roots(), path, version, name);
    if named.is_some() {
        announce_history(core, path);
    }
    named
}

/// Create a workspace folder under the same managed File Gate guard as note
/// creation. Do not add parent directories or overwrite an existing path.
/// This serializes ordinary Akula folder creation; it is not a sync-write API.
pub fn create_folder(core: &Core, path: &Path) -> Result<(), FileCommandError> {
    use std::io::ErrorKind;
    let _guard = core.file_mutation_guard_for_write()?;
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists =>
            Err(FileCommandError::AlreadyExists {
                path: path.to_string_lossy().into_owned(),
            }),
        Err(error) => Err(error.into()),
    }
}

/// Create a missing template directory tree under the Core File Gate.
/// Keep create_dir_all semantics for native starter templates: parents can be
/// created, an existing directory is accepted, and an existing file fails.
/// This is process-local serialization only, not root confinement or a
/// handle-relative sync mutation authority.
pub fn ensure_folder_tree(core: &Core, path: &Path) -> Result<(), FileCommandError> {
    let _guard = core.file_mutation_guard_for_write()?;
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod folder_tests {
    use super::*;
    use crate::app_core::{Core, CoreEvent};
    use std::fs;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn core_file_gate_serializes_folder_creation_without_clobber() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let app_data = temp.path().join("appdata");
        fs::create_dir(&vault).unwrap();
        fs::create_dir(&app_data).unwrap();
        let core = Core::open(&app_data, Arc::new(|_: CoreEvent| {}));
        let folder = vault.join("Новая папка 😀");

        let held = core.file_mutations.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let other_core = Arc::clone(&core);
        let target = folder.clone();
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_tx.send(create_folder(&other_core, &target)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "folder creation bypassed the Core File Gate lock");
        assert!(!folder.exists());
        drop(held);

        done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        worker.join().unwrap();
        assert!(folder.is_dir());
        assert!(matches!(
            create_folder(&core, &folder),
            Err(FileCommandError::AlreadyExists { .. })
        ));
    }

    #[test]
    fn core_file_gate_serializes_template_folder_tree_across_core_instances() {
        let temp = tempfile::tempdir().unwrap();
        let vault = temp.path().join("vault");
        let app_a = temp.path().join("app-a");
        let app_b = temp.path().join("app-b");
        for path in [&vault, &app_a, &app_b] {
            fs::create_dir(path).unwrap();
        }
        let first = Core::open(&app_a, Arc::new(|_: CoreEvent| {}));
        let second = Core::open(&app_b, Arc::new(|_: CoreEvent| {}));
        let templates = vault.join("Шаблоны 😀").join("Nested");
        let expected = templates.clone();

        let held = first.file_mutations.lock().unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            ready_tx.send(()).unwrap();
            done_tx.send(ensure_folder_tree(&second, &templates)).unwrap();
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(
            done_rx.recv_timeout(Duration::from_millis(150)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ), "template folder creation bypassed the process-wide File Gate");
        assert!(!expected.parent().unwrap().exists());
        drop(held);

        done_rx.recv_timeout(Duration::from_secs(30)).unwrap().unwrap();
        worker.join().unwrap();
        assert!(expected.is_dir());
        let existing = expected.join("Keep.md");
        fs::write(&existing, b"unchanged").unwrap();
        ensure_folder_tree(&first, &expected).unwrap();
        assert_eq!(fs::read(&existing).unwrap(), b"unchanged");
        assert!(ensure_folder_tree(&first, &existing).is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"unchanged");
    }
}
