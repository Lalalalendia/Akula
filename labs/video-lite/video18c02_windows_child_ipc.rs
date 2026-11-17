//! VIDEO-18c-02a: real Windows parent/child HANDLE allowlist and receipt lab.
//! Disposable GitHub-hosted PR only; this is NOT a first-frame witness, updater,
//! signature verifier, journal writer, update confirmation or release component.
//! This binary only starts a second instance of itself under a pinned OS handle.

#[cfg(not(windows))]
fn main() {
    eprintln!("VIDEO18C02_HOLD: Windows runner only");
    std::process::exit(2);
}

#[cfg(windows)]
mod lab {
    use std::ffi::{c_void, OsStr};
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};

    const PIPE: u32 = 3;
    const INHERIT: u32 = 1;
    const ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const WAIT_OBJECT_0: u32 = 0;
    const DEADLINE_MS: u32 = 15_000;
    const HEADER: &str = "AKULA-WIN-CHILD-IPC-LAB-V1";
    const VERSION: &str = "0.3.21";
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[repr(C)]
    struct SecurityAttributes {
        len: u32, descriptor: *mut c_void, inherit: i32,
    }
    #[repr(C)]
    struct StartupInfoW {
        cb: u32, reserved: *mut u16, desktop: *mut u16, title: *mut u16,
        x: u32, y: u32, x_size: u32, y_size: u32,
        x_chars: u32, y_chars: u32, fill: u32, flags: u32,
        show: u16, reserved2_bytes: u16, reserved2: *mut u8,
        stdin: *mut c_void, stdout: *mut c_void, stderr: *mut c_void,
    }
    #[repr(C)]
    struct StartupInfoExW {
        startup: StartupInfoW, attributes: *mut c_void,
    }
    #[repr(C)]
    struct ProcessInformation {
        process: *mut c_void, thread: *mut c_void, pid: u32, tid: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(read: *mut *mut c_void, write: *mut *mut c_void,
            attrs: *mut SecurityAttributes, size: u32) -> i32;
        fn SetHandleInformation(handle: *mut c_void, mask: u32, value: u32) -> i32;
        fn GetHandleInformation(handle: *mut c_void, flags: *mut u32) -> i32;
        fn GetFileType(handle: *mut c_void) -> u32;
        fn GetCurrentProcessId() -> u32;
        fn GetProcessId(handle: *mut c_void) -> u32;
        fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32,
            flags: u32, bytes: *mut usize) -> i32;
        fn UpdateProcThreadAttribute(list: *mut c_void, flags: u32,
            attr: usize, value: *mut c_void, size: usize,
            previous: *mut c_void, result_size: *mut usize) -> i32;
        fn DeleteProcThreadAttributeList(list: *mut c_void);
        fn CreateProcessW(application: *const u16, command: *mut u16,
            process_attrs: *mut SecurityAttributes, thread_attrs: *mut SecurityAttributes,
            inherit: i32, flags: u32, environment: *mut c_void,
            directory: *const u16, startup: *mut StartupInfoW,
            process: *mut ProcessInformation) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
        fn GetExitCodeProcess(handle: *mut c_void, code: *mut u32) -> i32;
        fn TerminateProcess(handle: *mut c_void, code: u32) -> i32;
        fn ReadFile(handle: *mut c_void, buffer: *mut c_void, bytes: u32,
            read: *mut u32, overlapped: *mut c_void) -> i32;
        fn WriteFile(handle: *mut c_void, buffer: *const c_void, bytes: u32,
            written: *mut u32, overlapped: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8,
            bytes: u32, flags: u32) -> i32;
    }

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0); }
            }
        }
    }

    fn new_pipe() -> Result<(Handle, Handle), String> {
        let mut attrs = SecurityAttributes {
            len: size_of::<SecurityAttributes>() as u32,
            descriptor: null_mut(),
            inherit: 1,
        };
        let mut reader = null_mut();
        let mut writer = null_mut();
        if unsafe { CreatePipe(&mut reader, &mut writer, &mut attrs, 0) } == 0 {
            return Err("CreatePipe failed".into());
        }
        let pair = (Handle(reader), Handle(writer));
        if unsafe { SetHandleInformation(pair.0.0, INHERIT, 0) } == 0 {
            return Err("read-end must not be inheritable".into());
        }
        Ok(pair)
    }

    fn random_nonce() -> Result<String, String> {
        let mut data = [0u8; 16];
        if unsafe { BCryptGenRandom(null_mut(), data.as_mut_ptr(), 16, 2) } != 0 {
            return Err("OS CSPRNG failure".into());
        }
        Ok(data.iter().map(|b| format!("{b:02x}")).collect())
    }

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    fn child(args: &[String]) -> i32 {
        if args.len() != 8 { return 20; }
        let Ok(write) = args[2].parse::<usize>() else { return 21 };
        let Ok(decoy) = args[3].parse::<usize>() else { return 22 };
        let pipe = write as *mut c_void;
        let unrelated = decoy as *mut c_void;
        let mut flags = 0u32;
        if unsafe { GetFileType(pipe) } != PIPE
            || unsafe { GetHandleInformation(pipe, &mut flags) } == 0
            || flags & INHERIT == 0
        {
            return 23;
        }
        // A second intentionally inheritable HANDLE must NOT cross the
        // STARTUPINFOEX handle whitelist, even when its value is supplied.
        if unsafe { GetFileType(unrelated) } == PIPE { return 24; }
        let nonce = if args[7] == "wrong-nonce" {
            if args[4].bytes().all(|b| b == b'0') {
                "11111111111111111111111111111111".to_string()
            } else {
                "00000000000000000000000000000000".to_string()
            }
        } else {
            args[4].clone()
        };
        if args[7] == "silent" { return 0; }
        let receipt = format!(
            "{HEADER}\t{nonce}\t{}\t{}\t{}\n",
            args[6], args[5], unsafe { GetCurrentProcessId() }
        );
        if receipt.len() > 256 { return 25; }
        let repeat = if args[7] == "duplicate" { 2 } else { 1 };
        for _ in 0..repeat {
            let mut sent = 0u32;
            if unsafe { WriteFile(pipe, receipt.as_ptr().cast(),
                receipt.len() as u32, &mut sent, null_mut()) } == 0
                || sent as usize != receipt.len()
            {
                return 26;
            }
        }
        0
    }

    fn run_case(mode: &str, nonce: &str) -> Result<bool, String> {
        let (reader, writer) = new_pipe()?;
        let (_decoy_reader, decoy_writer) = new_pipe()?;
        let mut bytes = 0usize;
        unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut bytes); }
        if bytes == 0 { return Err("no attribute-list size".into()); }
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = storage.as_mut_ptr().cast::<c_void>();
        if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) } == 0 {
            return Err("InitializeProcThreadAttributeList failed".into());
        }
        let mut allowed = [writer.0];
        let added = unsafe {
            UpdateProcThreadAttribute(list, 0, ATTRIBUTE_HANDLE_LIST,
                allowed.as_mut_ptr().cast(), size_of::<*mut c_void>(),
                null_mut(), null_mut())
        };
        if added == 0 {
            unsafe { DeleteProcThreadAttributeList(list); }
            return Err("HANDLE_LIST failed".into());
        }
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let app = wide(executable.as_os_str());
        let line = format!("\"{}\" --child {} {} {} {} {} {}",
            executable.display(), writer.0 as usize, decoy_writer.0 as usize,
            nonce, DIGEST, VERSION, mode);
        let mut command = wide(OsStr::new(&line));
        let mut startup: StartupInfoExW = unsafe { zeroed() };
        startup.startup.cb = size_of::<StartupInfoExW>() as u32;
        startup.attributes = list;
        let mut info: ProcessInformation = unsafe { zeroed() };
        let launched = unsafe {
            CreateProcessW(app.as_ptr(), command.as_mut_ptr(), null_mut(),
                null_mut(), 1, EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW,
                null_mut(), null(), &mut startup.startup, &mut info)
        };
        unsafe { DeleteProcThreadAttributeList(list); }
        if launched == 0 { return Err("CreateProcessW failed".into()); }
        let process = Handle(info.process);
        let _thread = Handle(info.thread);
        drop(writer); // Parent must not keep a writer that conceals child EOF.
        if info.pid <= 1 || unsafe { GetProcessId(process.0) } != info.pid {
            return Err("untrusted child PID/OS process identity".into());
        }
        let waited = unsafe { WaitForSingleObject(process.0, DEADLINE_MS) };
        if waited != WAIT_OBJECT_0 {
            unsafe {
                TerminateProcess(process.0, 1);
                WaitForSingleObject(process.0, 2_000);
            }
            return Err("child did not exit inside deadline".into());
        }
        let mut exit = u32::MAX;
        if unsafe { GetExitCodeProcess(process.0, &mut exit) } == 0 {
            return Err("child exit cannot be verified".into());
        }
        let mut all = Vec::new();
        loop {
            let mut part = [0u8; 256];
            let mut got = 0u32;
            let ok = unsafe { ReadFile(reader.0, part.as_mut_ptr().cast(), 256,
                &mut got, null_mut()) };
            if ok == 0 || got == 0 { break; } // Child exited and pipe is closed.
            all.extend_from_slice(&part[..got as usize]);
            if all.len() > 256 { break; }
        }
        let expected = format!(
            "{HEADER}\t{nonce}\t{VERSION}\t{DIGEST}\t{}\n", info.pid
        );
        Ok(exit == 0 && all.len() <= 256 && all == expected.as_bytes())
    }

    fn hosted_pr_only() -> bool {
        [
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("RUNNER_OS", "Windows"),
            ("RUNNER_ENVIRONMENT", "github-hosted"),
        ].iter().all(|(key, value)| matches!(std::env::var(key), Ok(v) if v == *value))
    }

    pub fn main() {
        if !hosted_pr_only() {
            eprintln!("VIDEO18C02_HOLD: only disposable hosted Windows pull_request");
            std::process::exit(2);
        }
        let args: Vec<String> = std::env::args().collect();
        if args.get(1).is_some_and(|s| s == "--child") {
            std::process::exit(child(&args));
        }
        if args.len() != 1 {
            eprintln!("VIDEO18C02_HOLD: invalid lab mode");
            std::process::exit(3);
        }
        let nonce = random_nonce().expect("OS CSPRNG");
        assert_eq!(nonce.len(), 32);
        assert!(nonce.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        for (mode, accepted) in [
            ("good", true),
            ("wrong-nonce", false),
            ("duplicate", false),
            ("silent", false),
        ] {
            let result = run_case(mode, &nonce)
                .unwrap_or_else(|e| panic!("VIDEO18C02_HOLD {mode}: {e}"));
            assert_eq!(result, accepted,
                "VIDEO18C02_HOLD: mode {mode} has wrong process-bound result");
        }
        println!(
            "VIDEO18C02_REAL_PARENT_CHILD_PIPE_PASS pinned_child=1 \
             handle_allowlist=1 os_nonce=1 wrong_nonce_denied=1 \
             duplicate_denied=1 silent_denied=1 bounded_wait=1 \
             real_gui=0 signed_generation=0 confirm=0 release=0"
        );
    }
}

#[cfg(windows)]
fn main() { lab::main(); }
