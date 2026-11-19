//! VIDEO-TOOLS-01g: hold protected package bytes via native Windows FILE_SHARE_READ handles.
//! Diagnostic/read-only only. This module never launches an executable or exports a path.
#![allow(dead_code)]
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

const FILE_SHARE_READ: u32 = 0x0000_0001;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
pub(crate) const LAB_NAMES: [&str; 4] = [
    "akula-video-mux.exe", "akula-yt-dlp.exe", "akula-quickjs.exe", "uninstall.exe",
];
pub(crate) const SIGNED_NAMES: [&str; 6] = [
    "akula-video-mux.exe", "akula-yt-dlp.exe", "akula-quickjs.exe",
    "manifest.json", "manifest.minisig", "uninstall.exe",
];

fn hold(reason: &str) -> String {
    format!("VIDEO_TOOLS_01G_HOLD: {reason}")
}
fn reparse_metadata(path: &Path, directory: bool) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|_| hold("package path unavailable"))?;
    if meta.file_attributes() & 0x400 != 0
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(hold("reparse or wrong-type package object"));
    }
    Ok(())
}
fn safe_ancestors(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err(hold("package root is not absolute"));
    }
    for part in root.ancestors() {
        reparse_metadata(part, true)?;
    }
    Ok(())
}
fn guarded_open(path: &Path, directory: bool) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true).share_mode(FILE_SHARE_READ).custom_flags(
        FILE_FLAG_OPEN_REPARSE_POINT |
            if directory { FILE_FLAG_BACKUP_SEMANTICS } else { 0 }
    );
    let file = options.open(path).map_err(|_| hold("cannot acquire held read-only handle"))?;
    let meta = file.metadata().map_err(|_| hold("cannot stat held handle"))?;
    if meta.file_attributes() & 0x400 != 0
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(hold("held handle targets reparse or wrong file type"));
    }
    reparse_metadata(path, directory)?;
    Ok(file)
}

/// Owned root and every component handle; each share mode is FILE_SHARE_READ
/// (no write or delete sharing). Dropping the guard releases all locks.
/// Caller MUST already bind the root to OS-known Program Files and check ACL.
/// No arbitrary caller-selected executable may become an admitted package.
pub(crate) struct PinnedPackage {
    _root: File,
    files: Vec<(&'static str, File)>,
}
impl PinnedPackage {
    pub(crate) fn open(root: &Path, names: &'static [&'static str]) -> Result<Self, String> {
        if names != LAB_NAMES.as_slice() && names != SIGNED_NAMES.as_slice() {
            return Err(hold("unknown package identity or path selection"));
        }
        safe_ancestors(root)?;
        let root_handle = guarded_open(root, true)?;
        let mut count = 0usize;
        for entry in fs::read_dir(root).map_err(|_| hold("cannot enumerate held root"))? {
            let entry = entry.map_err(|_| hold("cannot enumerate package object"))?;
            count += 1;
            if count > names.len() || !names.iter().any(|name| entry.file_name() == OsStr::new(name)) {
                return Err(hold("unexpected executable or sidecar in held root"));
            }
            reparse_metadata(&entry.path(), false)?;
        }
        if count != names.len() {
            return Err(hold("incomplete held package identity"));
        }
        let mut files = Vec::with_capacity(names.len());
        for &name in names {
            files.push((name, guarded_open(&root.join(name), false)?));
        }
        safe_ancestors(root)?;
        Ok(Self { _root: root_handle, files })
    }

    /// The callback receives the SAME open file used to hold the read-only,
    /// deny-write-and-delete sharing contract. It must authenticate the bytes.
    /// No future CreateProcess permission or executable pathname is returned.
    pub(crate) fn inspect(
        &mut self,
        mut authenticate: impl FnMut(&str, &mut File) -> Result<(), String>,
    ) -> Result<(), String> {
        for (name, file) in &mut self.files {
            file.seek(SeekFrom::Start(0)).map_err(|_| hold("cannot rewind held file"))?;
            authenticate(name, file)?;
        }
        Ok(())
    }
}

#[cfg(any(test, akula_video_tools_01g_witness))]
fn compare_reference(held: &mut File, reference: &mut File) -> Result<(), String> {
    let mut current = [0u8; 64 * 1024];
    let mut expected = [0u8; 64 * 1024];
    loop {
        let n = held.read(&mut current).map_err(|_| hold("held bytes unreadable"))?;
        let m = reference.read(&mut expected).map_err(|_| hold("reference bytes unreadable"))?;
        if n != m || current[..n] != expected[..m] {
            return Err(hold("held installed bytes differ from pinned source"));
        }
        if n == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    #[test]
    fn read_only_handles_block_mutation_until_guard_dropped() {
        let root = std::env::temp_dir().join(format!(
            "akula-01g-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .unwrap().as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        for name in LAB_NAMES {
            fs::write(root.join(name), b"MZ-akula-non-release-witness").unwrap();
        }
        let mut pinned = PinnedPackage::open(&root, &LAB_NAMES).unwrap();
        pinned.inspect(|_, file| {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if bytes != b"MZ-akula-non-release-witness" {
                return Err(hold("fixture bytes differ"));
            }
            Ok(())
        }).unwrap();
        let denied = OpenOptions::new().write(true).open(root.join(LAB_NAMES[0]))
            .expect_err("pinned FILE_SHARE_READ handle must deny write");
        assert_eq!(denied.raw_os_error(), Some(32), "expected sharing violation");
        drop(pinned);
        OpenOptions::new().write(true).open(root.join(LAB_NAMES[0]))
            .expect("write opens again once pinning guard is dropped");
        fs::write(root.join("alien.exe"), b"alien").unwrap();
        assert!(PinnedPackage::open(&root, &LAB_NAMES).is_err());
        fs::remove_file(root.join("alien.exe")).unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reference_comparison_rejects_tampered_bytes() {
        let root = std::env::temp_dir().join(format!(
            "akula-01g-negative-{}", std::process::id()
        ));
        let _ = fs::remove_file(&root);
        fs::write(&root, b"MZ-unchanged").unwrap();
        let mut held = File::open(&root).unwrap();
        // The source reference is independent of the installed payload.
        let reference = root.with_extension("reference");
        fs::write(&reference, b"MZ-original").unwrap();
        let mut expected = File::open(&reference).unwrap();
        assert!(compare_reference(&mut held, &mut expected).is_err());
        fs::remove_file(&root).unwrap();
        fs::remove_file(&reference).unwrap();
    }
}

#[cfg(akula_video_tools_01g_witness)]
fn main() {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    let root = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01G_ROOT")
        .expect("VIDEO_TOOLS_01G_HOLD: non-release NSIS root required"));
    let source = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01G_SOURCE")
        .expect("VIDEO_TOOLS_01G_HOLD: pinned PE staging directory required"));
    let work = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01G_WORK")
        .expect("VIDEO_TOOLS_01G_HOLD: disposable witness work directory required"));
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("RUNNER_OS").as_deref() != Ok("Windows")
        || std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request")
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
    {
        panic!("VIDEO_TOOLS_01G_HOLD: hosted Windows PR only");
    }
    let mut guard = PinnedPackage::open(&root, &LAB_NAMES).expect("pin real installed PE handles");
    guard.inspect(|name, held| {
        let reference_path = if name == "uninstall.exe" {
            root.join(name) // The lab installer generates this separately.
        } else {
            source.join(name)
        };
        let mut reference = File::open(reference_path)
            .map_err(|_| hold("staged source reference unavailable"))?;
        compare_reference(held, &mut reference)
    }).expect("all held PE bytes must match exact staged source");
    fs::write(work.join("video-tools-01g-ready"), b"held").unwrap();
    let deadline = Instant::now() + Duration::from_secs(25);
    while !work.join("video-tools-01g-release").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(work.join("video-tools-01g-release").exists(),
        "VIDEO_TOOLS_01G_HOLD: race witness release did not arrive before deadline");
    drop(guard);
    println!("VIDEO_TOOLS_01G_NATIVE_PIN_PASS source_bytes=3 root_held=1 files_held=4 release=0");
}
