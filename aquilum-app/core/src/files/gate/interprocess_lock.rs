//! Cooperative, per-user interprocess serialization for File Gate mutations.
//!
//! The lock file lives in the OS temporary directory so independent Akula
//! profiles on the same OS user share one lock. It is never removed: unlinking
//! a held lock file could let another process lock a replacement inode.
//! This is a coordination lock, not an authorization boundary against a
//! hostile process running as the same user.

use std::fs::File;
use std::fs::TryLockError;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

fn lock_path() -> PathBuf {
    let temporary = std::env::temp_dir();
    #[cfg(windows)]
    let user_scope = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("USERNAME"));
    #[cfg(not(windows))]
    let user_scope = std::env::var_os("HOME").or_else(|| std::env::var_os("USER"));
    let user_scope = user_scope.unwrap_or_else(|| temporary.as_os_str().to_os_string());
    let key = blake3::hash(user_scope.to_string_lossy().as_bytes()).to_hex();
    temporary.join(format!("akula-filegate-v1-{key}.lock"))
}

fn refused(message: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message.to_owned())
}

#[cfg(unix)]
fn open_lock_file(path: &std::path::Path) -> io::Result<File> {
    use rustix::fs::{open, Mode, OFlags};
    use std::os::unix::fs::MetadataExt;

    let descriptor = open(
        path,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|error| {
        io::Error::new(
            ErrorKind::PermissionDenied,
            format!("cannot open File Gate lock: {error:?}"),
        )
    })?;
    let file = File::from(descriptor);
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.len() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o077 != 0
    {
        return Err(refused(
            "File Gate lock is not a private, empty regular file",
        ));
    }
    Ok(file)
}

#[cfg(windows)]
fn open_lock_file(path: &std::path::Path) -> io::Result<File> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    // Denying FILE_SHARE_DELETE keeps the named object stable while any
    // process holds it; read/write sharing allows competing lock attempts.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = file.metadata()?;
    let (_, _, hard_links) = windows_file_identity(&file)?;
    if !metadata.is_file()
        || metadata.len() != 0
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || hard_links != 1
    {
        return Err(refused(
            "File Gate lock is not a stable, empty regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_lock_file(_path: &std::path::Path) -> io::Result<File> {
    Err(refused(
        "interprocess File Gate locking is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn same_file(left: &File, right: &File) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let left = left.metadata()?;
    let right = right.metadata()?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

#[cfg(windows)]
fn same_file(left: &File, right: &File) -> io::Result<bool> {
    let (left_volume, left_index, _) = windows_file_identity(left)?;
    let (right_volume, right_index, _) = windows_file_identity(right)?;
    Ok(left_volume == right_volume && left_index == right_index)
}

#[cfg(windows)]
fn windows_file_identity(file: &File) -> io::Result<(u32, u64, u32)> {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        created: FileTime,
        accessed: FileTime,
        written: FileTime,
        volume_serial: u32,
        size_high: u32,
        size_low: u32,
        hard_links: u32,
        file_index_high: u32,
        file_index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(
            file: *mut c_void,
            information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut information = ByHandleFileInformation::default();
    // SAFETY: `file` is live, and the Win32 API writes into the initialized
    // structure provided here.
    let succeeded = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let index =
        (u64::from(information.file_index_high) << 32) | u64::from(information.file_index_low);
    if information.volume_serial == 0 || index == 0 || information.hard_links == 0 {
        return Err(refused(
            "File Gate lock has no stable Windows file identity",
        ));
    }
    Ok((information.volume_serial, index, information.hard_links))
}

#[cfg(not(any(unix, windows)))]
fn same_file(_left: &File, _right: &File) -> io::Result<bool> {
    Err(refused(
        "interprocess File Gate identity checks are unsupported on this platform",
    ))
}

fn confirm_named_object(file: &File) -> io::Result<()> {
    let named = open_lock_file(&lock_path())?;
    if !same_file(file, &named)? {
        return Err(refused(
            "File Gate lock pathname no longer names the locked object",
        ));
    }
    Ok(())
}

/// An exclusive lock held until this value is dropped.
pub(crate) struct FileGateInterprocessLock {
    _file: File,
}

impl FileGateInterprocessLock {
    pub(crate) fn acquire() -> io::Result<Self> {
        let file = open_lock_file(&lock_path())?;
        file.lock()?;
        confirm_named_object(&file)?;
        Ok(Self { _file: file })
    }

    pub(crate) fn try_acquire() -> io::Result<Option<Self>> {
        let file = open_lock_file(&lock_path())?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(error)) => return Err(error),
        }
        confirm_named_object(&file)?;
        Ok(Some(Self { _file: file }))
    }
}

#[cfg(test)]
pub(super) fn pause_file_gate_after_primary_before_history() {
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

#[cfg(test)]
mod tests {
    use crate::app_core::{Core, CoreEvent};
    use crate::files::gate;
    use crate::history::Source;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, ExitStatus};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    const CHILD_NAME: &str = "files::gate::interprocess_lock::tests::oslock_child_entrypoint";
    const CHILD_MODE: &str = "AKULA_FILEGATE_INTERPROCESS_TEST_MODE";
    const PAUSE_MARKER: &str = "AKULA_FILEGATE_TEST_PAUSE_MARKER";
    const PAUSE_RELEASE: &str = "AKULA_FILEGATE_TEST_PAUSE_RELEASE";
    const CHILD_CONTENT: &str = "AKULA_FILEGATE_TEST_CONTENT";

    struct KillChild(Child);

    impl Drop for KillChild {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }

    fn child_command(
        appdata: &Path,
        workspace: &Path,
        note: &Path,
        expected_hash: &str,
        report: &Path,
        content: &str,
        pause: Option<(&Path, &Path)>,
    ) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg(CHILD_NAME)
            .arg("--nocapture")
            .env(CHILD_MODE, "write")
            .env("AKULA_FILEGATE_TEST_APPDATA", appdata)
            .env("AKULA_FILEGATE_TEST_WORKSPACE", workspace)
            .env("AKULA_FILEGATE_TEST_NOTE", note)
            .env("AKULA_FILEGATE_TEST_EXPECTED_HASH", expected_hash)
            .env("AKULA_FILEGATE_TEST_REPORT", report)
            .env(CHILD_CONTENT, content);
        if let Some((marker, release)) = pause {
            command
                .env(PAUSE_MARKER, marker)
                .env(PAUSE_RELEASE, release);
        }
        command
    }

    fn wait_for_ready(child: &mut KillChild, report: &Path) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match fs::read(report) {
                Ok(bytes) if bytes == b"ready" => return,
                Ok(bytes) => panic!("unexpected child report before lock release: {bytes:?}"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot read child report: {error}"),
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "writer child exited before its ready signal"
            );
            thread::sleep(Duration::from_millis(10));
        }
        panic!("writer child did not become ready before deadline");
    }

    fn wait_for_exit(child: &mut KillChild) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Some(status) = child.0.try_wait().unwrap() {
                return status;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("writer child did not exit after File Gate lock release");
    }

    fn wait_for_primary_pause(child: &mut KillChild, marker: &Path) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match fs::read(marker) {
                Ok(bytes) if bytes == b"after-primary" => return,
                Ok(bytes) => panic!("unexpected File Gate pause marker: {bytes:?}"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => panic!("cannot read File Gate pause marker: {error}"),
            }
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "writer child exited before reaching its history pause"
            );
            thread::sleep(Duration::from_millis(10));
        }
        panic!("writer child did not reach its primary/history pause");
    }

    #[test]
    fn oslock_child_entrypoint() {
        if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("write")) {
            return;
        }
        let appdata = PathBuf::from(std::env::var_os("AKULA_FILEGATE_TEST_APPDATA").unwrap());
        let workspace = PathBuf::from(std::env::var_os("AKULA_FILEGATE_TEST_WORKSPACE").unwrap());
        let note = PathBuf::from(std::env::var_os("AKULA_FILEGATE_TEST_NOTE").unwrap());
        let expected_hash = std::env::var("AKULA_FILEGATE_TEST_EXPECTED_HASH").unwrap();
        let report = PathBuf::from(std::env::var_os("AKULA_FILEGATE_TEST_REPORT").unwrap());
        let content = std::env::var(CHILD_CONTENT).unwrap();

        let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
        core.ui_state
            .resolve_workspace(workspace.to_str().unwrap(), 1)
            .unwrap();
        fs::write(&report, b"ready").unwrap();
        let outcome = gate::write(
            &core,
            &note,
            &content,
            Some(&expected_hash),
            Source::Me,
            None,
        );
        match outcome {
            Ok(result) => fs::write(report, format!("written|{}", result.hash)).unwrap(),
            Err(error) => fs::write(report, format!("error|{error}")).unwrap(),
        }
    }

    #[test]
    fn two_process_filegate_writes_stay_serialized_through_history() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("shared-workspace");
        let first_appdata = temp.path().join("app-a");
        let second_appdata = temp.path().join("app-b");
        for path in [&workspace, &first_appdata, &second_appdata] {
            fs::create_dir(path).unwrap();
        }

        let first = Core::open(&first_appdata, Arc::new(|_: CoreEvent| {}));
        first
            .ui_state
            .resolve_workspace(workspace.to_str().unwrap(), 1)
            .unwrap();
        let note = workspace.join("Shared.md");
        let initial = gate::write(
            &first,
            &note,
            "baseline-before-writers",
            None,
            Source::Me,
            None,
        )
        .unwrap();
        let first_report = temp.path().join("child-a-report.txt");
        let second_report = temp.path().join("child-b-report.txt");
        let primary_pause = temp.path().join("child-a-after-primary.txt");
        let release_pause = temp.path().join("release-child-a.txt");

        // Child A pauses after publishing primary bytes and releasing the
        // process-local mutex, while retaining the OS lock before history.
        let mut first_child = KillChild(
            child_command(
                &first_appdata,
                &workspace,
                &note,
                &initial.hash,
                &first_report,
                "first-process-edit",
                Some((&primary_pause, &release_pause)),
            )
            .spawn()
            .unwrap(),
        );
        wait_for_ready(&mut first_child, &first_report);
        wait_for_primary_pause(&mut first_child, &primary_pause);
        let first_content_hash = blake3::hash(b"first-process-edit").to_hex().to_string();
        assert_eq!(fs::read(&note).unwrap(), b"first-process-edit");

        // A separate Akula process must wait even though the first process's
        // local mutex is already free: its OS lock spans the history gap.
        let mut second_child = KillChild(
            child_command(
                &second_appdata,
                &workspace,
                &note,
                &first_content_hash,
                &second_report,
                "second-process-edit",
                None,
            )
            .spawn()
            .unwrap(),
        );
        wait_for_ready(&mut second_child, &second_report);

        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            assert_eq!(
                fs::read(&second_report).unwrap(),
                b"ready",
                "second process wrote while the first history commit was paused"
            );
            assert_eq!(fs::read(&note).unwrap(), b"first-process-edit");
            assert!(
                second_child.0.try_wait().unwrap().is_none(),
                "second process did not wait for the first File Gate history commit"
            );
            thread::sleep(Duration::from_millis(10));
        }

        fs::write(&release_pause, b"continue").unwrap();
        assert!(wait_for_exit(&mut first_child).success());
        assert!(wait_for_exit(&mut second_child).success());

        let first_report_text = fs::read_to_string(&first_report).unwrap();
        assert!(
            first_report_text.starts_with("written|"),
            "first process failed its gated write: {first_report_text}"
        );
        assert_eq!(
            first_report_text.strip_prefix("written|").unwrap(),
            first_content_hash
        );
        let second_report_text = fs::read_to_string(&second_report).unwrap();
        assert!(
            second_report_text.starts_with("written|"),
            "second process failed its gated write: {second_report_text}"
        );
        assert_eq!(fs::read(&note).unwrap(), b"second-process-edit");
        assert_eq!(
            blake3::hash(&fs::read(&note).unwrap()).to_hex().as_str(),
            second_report_text.strip_prefix("written|").unwrap()
        );

        // Reusing the first process's stale version must not overwrite the
        // second process's successful edit.
        let stale = gate::write(
            &first,
            &note,
            "stale-overwrite",
            Some(&initial.hash),
            Source::Me,
            None,
        );
        assert!(
            stale.is_err(),
            "stale expected hash unexpectedly overwrote the note"
        );
        assert_eq!(fs::read(&note).unwrap(), b"second-process-edit");
        eprintln!(
            "PC-FG-01 baseline_blake3={} writer_a_blake3={} writer_b_blake3={} stale_write=refused",
            initial.hash,
            first_content_hash,
            second_report_text.strip_prefix("written|").unwrap(),
        );
    }
}
