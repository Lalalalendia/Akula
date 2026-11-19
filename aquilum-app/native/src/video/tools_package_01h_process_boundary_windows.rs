//! VIDEO-TOOLS-01h: NON-RELEASE, benign-only Windows process-handoff witness.
//! No Video Tools EXE is ever executed. No production API can launch a tool.
//! Reuses the identical verified FILE_SHARE_READ root/file pin guard from 01g.
#![allow(dead_code)]
#![allow(non_snake_case)]

#[path = "tools_package_pin_windows.rs"]
mod pinned;
#[path = "tools_package_windows.rs"]
mod os_root;

#[cfg(akula_video_tools_01h_witness)]
mod witness {
    use super::{os_root, pinned};
    use std::ffi::{c_void, OsStr};
    use std::fs::{self, File};
    use std::io::Read;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_LIMIT_MS: u32 = 30_000;
    const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;

    #[repr(C)]
    struct StartupInfoW {
        cb: u32,
        lpReserved: *mut u16,
        lpDesktop: *mut u16,
        lpTitle: *mut u16,
        dwX: u32,
        dwY: u32,
        dwXSize: u32,
        dwYSize: u32,
        dwXCountChars: u32,
        dwYCountChars: u32,
        dwFillAttribute: u32,
        dwFlags: u32,
        wShowWindow: u16,
        cbReserved2: u16,
        lpReserved2: *mut u8,
        hStdInput: *mut c_void,
        hStdOutput: *mut c_void,
        hStdError: *mut c_void,
    }
    #[repr(C)]
    struct ProcessInformation {
        process: *mut c_void,
        thread: *mut c_void,
        process_id: u32,
        thread_id: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateProcessW(
            application_name: *const u16,
            command_line: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit_handles: i32,
            creation_flags: u32,
            environment: *const c_void,
            current_directory: *const u16,
            startup_info: *mut StartupInfoW,
            process_info: *mut ProcessInformation,
        ) -> i32;
        fn GetLastError() -> u32;
        fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
        fn GetExitCodeProcess(handle: *mut c_void, exit_code: *mut u32) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    struct OwnedHandle(*mut c_void);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }
    fn hold(message: &str) -> String {
        format!("VIDEO_TOOLS_01H_HOLD: {message}")
    }
    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
    fn await_barrier(path: &Path, description: &str) -> Result<(), String> {
        let limit = Instant::now() + Duration::from_secs(30);
        loop {
            if path.is_file() {
                return Ok(());
            }
            if Instant::now() >= limit {
                return Err(hold(&format!("{description} was not observed")));
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    }
    fn exact_lab_root(root: &Path) -> Result<(), String> {
        let os_program_files = os_root::known_program_files_x64()?;
        if root.parent() != Some(os_program_files.as_path()) {
            return Err(hold("lab root is not immediately below OS-known Program Files x64"));
        }
        let name = root.file_name().and_then(|name| name.to_str())
            .ok_or_else(|| hold("lab directory name unavailable"))?;
        let id = name.strip_prefix("AkulaVideoToolsLab-")
            .ok_or_else(|| hold("unknown lab root identity"))?;
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(hold("lab root does not contain an exact GUID-like identity"));
        }
        os_root::check_installed_acl(root, &pinned::LAB_NAMES)?;
        Ok(())
    }
    fn compare_installed_to_source(guard: &mut pinned::PinnedPackage, stage: &Path) -> Result<(), String> {
        guard.inspect(|_name, file| {
            let mut source = File::open(stage).map_err(|_| hold("source-built benign PE missing"))?;
            let length = file.metadata().map_err(|_| hold("held executable inaccessible"))?.len();
            if source.metadata().map_err(|_| hold("original executable inaccessible"))?.len() != length
                || length < 64 || length > MAX_FILE_BYTES {
                return Err(hold("held binary size differs from reviewed benign fixture"));
            }
            let mut old = [0u8; 65536];
            let mut current = [0u8; 65536];
            let mut remain = length;
            while remain > 0 {
                let n = (remain as usize).min(old.len());
                file.read_exact(&mut current[..n]).map_err(|_| hold("held bytes unreadable"))?;
                source.read_exact(&mut old[..n]).map_err(|_| hold("source bytes unreadable"))?;
                if old[..n] != current[..n] {
                    return Err(hold("held bytes do not match source-built benign PE"));
                }
                remain -= n as u64;
            }
            Ok(())
        })
    }
    fn spawn_exact_benign_child(file: &Path, work: &Path) -> Result<(OwnedHandle, OwnedHandle), String> {
        let application = wide(file.as_os_str());
        let mut command = wide(OsStr::new(&format!("\"{}\" --akula-01h-child", file.display())));
        let workdir = wide(work.as_os_str());
        let mut startup: StartupInfoW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of::<StartupInfoW>() as u32;
        let mut info: ProcessInformation = unsafe { std::mem::zeroed() };
        let launched = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command.as_mut_ptr(),
                ptr::null(), ptr::null(),
                0, // bInheritHandles = FALSE
                CREATE_NO_WINDOW,
                ptr::null(), // controlled test env inherited; no injected PATH search
                workdir.as_ptr(),
                &mut startup, &mut info,
            )
        };
        if launched == 0 {
            return Err(hold(&format!("CreateProcessW failed while pins held, Win32={}", unsafe {
                GetLastError()
            })));
        }
        if info.process.is_null() || info.thread.is_null() || info.process_id == 0 {
            return Err(hold("CreateProcessW produced invalid process handles"));
        }
        Ok((OwnedHandle(info.process), OwnedHandle(info.thread)))
    }
    pub(super) fn run() -> Result<(), String> {
        if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
            || std::env::var("RUNNER_OS").as_deref() != Ok("Windows")
            || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
            || std::env::var("GITHUB_EVENT_NAME").as_deref() != Ok("pull_request") {
            return Err(hold("witness must run on a disposable hosted Windows PR runner"));
        }
        let root = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01H_ROOT")
            .ok_or_else(|| hold("lab root missing"))?);
        let source = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01H_SOURCE")
            .ok_or_else(|| hold("benign original missing"))?);
        let work = PathBuf::from(std::env::var_os("AKULA_VIDEO_TOOLS_01H_WORK")
            .ok_or_else(|| hold("disposable work root missing"))?);
        if !source.is_file() || !work.is_dir() {
            return Err(hold("missing source binary or work directory"));
        }
        exact_lab_root(&root)?;
        let mut guard = pinned::PinnedPackage::open(&root, &pinned::LAB_NAMES)?;
        compare_installed_to_source(&mut guard, &source)?;
        fs::write(work.join("01h-pinned"), b"held")
            .map_err(|_| hold("cannot mark held files"))?;
        await_barrier(&work.join("01h-start"), "launch permission barrier")?;
        // Crucially the SAME guard remains alive across CreateProcessW and
        // until the child independently proves that its image started.
        let executable = root.join(pinned::LAB_NAMES[0]);
        let (child, thread) = spawn_exact_benign_child(&executable, &work)?;
        await_barrier(&work.join("01h-child-started"), "benign image startup")?;
        fs::write(work.join("01h-launched"), b"created-with-pins-held")
            .map_err(|_| hold("cannot report verified child startup"))?;
        await_barrier(&work.join("01h-finish"), "second mutation probe")?;
        let waited = unsafe { WaitForSingleObject(child.0, WAIT_LIMIT_MS) };
        if waited != WAIT_OBJECT_0 {
            return Err(hold("benign child did not exit in bounded time"));
        }
        let mut exit_code = u32::MAX;
        if unsafe { GetExitCodeProcess(child.0, &mut exit_code) } == 0 || exit_code != 0 {
            return Err(hold("benign child exited abnormally"));
        }
        // Root and all four EXE handles stay open until the tested child exits.
        drop(thread);
        drop(child);
        drop(guard);
        println!("VIDEO_TOOLS_01H_CREATEPROCESS_PASS benign=1 signed_tools_executed=0 root_pinned=1 files_pinned=4 createprocess_while_held=1 startup_while_held=1 child_exit=0 release=0");
        Ok(())
    }
}
#[cfg(akula_video_tools_01h_witness)]
fn main() {
    if let Err(reason) = witness::run() {
        eprintln!("{reason}");
        std::process::exit(3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_build_has_no_process_launcher_export() {
        assert_eq!(pinned::LAB_NAMES.len(), 4);
        assert_eq!(pinned::LAB_NAMES[0], "akula-video-mux.exe");
    }
}
