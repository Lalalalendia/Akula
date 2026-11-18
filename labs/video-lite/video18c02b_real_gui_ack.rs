//! VIDEO-18c-02b-01: actual Akula first-presented-frame ACK from protected current/.
//! NON-RELEASE: disposable hosted Windows pull_request only. No update, no
//! signed generation, no journal, no confirm_candidate, no publisher key.
//! This is a separate OS parent launching the real Akula GUI, not a fake child.
#[cfg(not(windows))]
fn main() {
    eprintln!("VIDEO18C02B_HOLD: Windows-only laboratory");
    std::process::exit(2);
}
#[cfg(windows)]
pub(crate) mod lab {
    use std::ffi::{c_void, OsStr};
    use std::fs::{self, File};
    use std::io::Read;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::{Path, PathBuf};
    use std::ptr::{null, null_mut};
    use std::time::{Duration, Instant};

    const PIPE_TYPE: u32 = 3;
    const HANDLE_FLAG_INHERIT: u32 = 1;
    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
    const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
    const WAIT_OBJECT_0: u32 = 0;
    const STILL_ACTIVE: u32 = 259;
    const WM_CLOSE: u32 = 0x0010;
    const MAX_RECEIPT: usize = 256;
    const FRAME_DEADLINE: Duration = Duration::from_secs(75);
    const HEADER: &str = "AKULA-WIN-FIRST-PRESENTED-V1";
    const VERSION: &str = "0.3.21"; // Test generation only, NOT a signed release.
    const REQUIRED: [&str; 5] = [
        "akula-app.exe", "shiva-pdf-preview.exe", "akula-video-mux.exe",
        "akula-yt-dlp.exe", "akula-quickjs.exe",
    ];
    #[repr(C)]
    struct SecurityAttributes { len: u32, descriptor: *mut c_void, inherit: i32 }
    #[repr(C)]
    struct StartupInfoW {
        cb: u32, reserved: *mut u16, desktop: *mut u16, title: *mut u16,
        x: u32, y: u32, x_size: u32, y_size: u32,
        x_chars: u32, y_chars: u32, fill: u32, flags: u32,
        show: u16, reserved2_bytes: u16, reserved2: *mut u8,
        stdin: *mut c_void, stdout: *mut c_void, stderr: *mut c_void,
    }
    #[repr(C)]
    struct StartupInfoExW { startup: StartupInfoW, attributes: *mut c_void }
    #[repr(C)]
    struct ProcessInformation {
        process: *mut c_void, thread: *mut c_void, pid: u32, tid: u32,
    }
    #[repr(C)]
    #[derive(Clone, Copy, PartialEq, Eq)]
    struct FileTime { low: u32, high: u32 }
    #[repr(C)]
    #[derive(Default)]
    struct Rect { left: i32, top: i32, right: i32, bottom: i32 }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(reader: *mut *mut c_void, writer: *mut *mut c_void,
            attrs: *mut SecurityAttributes, size: u32) -> i32;
        fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
        fn GetHandleInformation(handle: *mut c_void, flags: *mut u32) -> i32;
        fn GetFileType(handle: *mut c_void) -> u32;
        fn GetCurrentProcessId() -> u32;
        fn GetProcessId(process: *mut c_void) -> u32;
        fn GetProcessTimes(process: *mut c_void, creation: *mut FileTime,
            exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn InitializeProcThreadAttributeList(list: *mut c_void,
            count: u32, flags: u32, bytes: *mut usize) -> i32;
        fn UpdateProcThreadAttribute(list: *mut c_void, flags: u32,
            attribute: usize, value: *mut c_void, bytes: usize,
            previous: *mut c_void, result: *mut usize) -> i32;
        fn DeleteProcThreadAttributeList(list: *mut c_void);
        fn CreateProcessW(application: *const u16, command: *mut u16,
            process_attrs: *mut SecurityAttributes, thread_attrs: *mut SecurityAttributes,
            inherit: i32, flags: u32, environment: *mut c_void,
            directory: *const u16, startup: *mut StartupInfoW,
            process: *mut ProcessInformation) -> i32;
        fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, timeout: u32) -> u32;
        fn TerminateProcess(handle: *mut c_void, exit: u32) -> i32;
        fn PeekNamedPipe(pipe: *mut c_void, buffer: *mut c_void, size: u32,
            read: *mut u32, available: *mut u32, remaining: *mut u32) -> i32;
        fn ReadFile(pipe: *mut c_void, buffer: *mut c_void, size: u32,
            read: *mut u32, overlapped: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(algorithm: *mut c_void, buffer: *mut u8,
            bytes: u32, flags: u32) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(*mut c_void, isize) -> i32,
            param: isize) -> i32;
        fn GetWindowThreadProcessId(window: *mut c_void, pid: *mut u32) -> u32;
        fn IsWindowVisible(window: *mut c_void) -> i32;
        fn GetWindowRect(window: *mut c_void, rect: *mut Rect) -> i32;
        fn GetWindowTextW(window: *mut c_void, buffer: *mut u16, max: i32) -> i32;
        fn PostMessageW(window: *mut c_void, message: u32, wparam: usize,
            lparam: isize) -> i32;
    }

    struct Handle(*mut c_void);
    impl Drop for Handle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0); }
            }
        }
    }
    struct Child(Handle);
    impl Drop for Child {
        fn drop(&mut self) {
            // Never leave a test GUI running when a receipt or window check fails.
            if unsafe { WaitForSingleObject(self.0.0, 0) } != WAIT_OBJECT_0 {
                unsafe { TerminateProcess(self.0.0, 1); }
                unsafe { WaitForSingleObject(self.0.0, 5_000); }
            }
        }
    }
    struct Witness { pid: u32, window: *mut c_void }
    unsafe extern "system" fn window_callback(hwnd: *mut c_void, pointer: isize) -> i32 {
        let witness = unsafe { &mut *(pointer as *mut Witness) };
        let mut owner = 0u32;
        if unsafe { GetWindowThreadProcessId(hwnd, &mut owner) } == 0
            || owner != witness.pid || unsafe { IsWindowVisible(hwnd) } == 0
        { return 1; }
        let mut r = Rect::default();
        if unsafe { GetWindowRect(hwnd, &mut r) } == 0
            || r.right - r.left < 400 || r.bottom - r.top < 300
        { return 1; }
        let mut name = [0u16; 256];
        let count = unsafe { GetWindowTextW(hwnd, name.as_mut_ptr(), 256) };
        if count <= 0 || !String::from_utf16_lossy(&name[..count as usize]).contains("Akula") {
            return 1;
        }
        witness.window = hwnd;
        0
    }
    fn visible_window(pid: u32) -> Option<*mut c_void> {
        let mut w = Witness { pid, window: null_mut() };
        unsafe { EnumWindows(window_callback, (&mut w as *mut Witness) as isize); }
        if w.window.is_null() { None } else { Some(w.window) }
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }
    fn lower_hex(s: &str, len: usize) -> bool {
        s.len() == len && s.bytes().all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
    }
    fn validate(path: &Path, directory: bool) -> Result<(), String> {
        let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if m.file_attributes() & 0x400 != 0 || m.file_type().is_symlink()
            || if directory { !m.is_dir() } else { !m.is_file() }
        { return Err(format!("reparse/missing/unexpected path: {}", path.display())); }
        Ok(())
    }
    pub(crate) fn validate_app(raw: &str) -> Result<PathBuf, String> {
        let requested = PathBuf::from(raw);
        validate(&requested, false)?;
        let app = fs::canonicalize(&requested).map_err(|e| e.to_string())?;
        if app.file_name().and_then(|x| x.to_str()) != Some("akula-app.exe") {
            return Err("only actual current/akula-app.exe permitted".into());
        }
        let current = app.parent().ok_or("no current/ parent")?;
        validate(current, true)?;
        if current.file_name().and_then(|x| x.to_str()) != Some("current") {
            return Err("not a versioned protected current/".into());
        }
        let root = current.parent().ok_or("no protected root")?;
        validate(root, true)?;
        let leaf = root.file_name().and_then(|x| x.to_str()).ok_or("bad protected root")?;
        let tag = leaf.strip_prefix("AkulaVideo18bRealNsisLab-")
            .ok_or("unexpected NSIS lab root")?;
        if !lower_hex(tag, 32) {
            return Err("not a fresh GUID-only protected root".into());
        }
        let program_files = fs::canonicalize(std::env::var_os("ProgramFiles")
            .ok_or("missing ProgramFiles")?).map_err(|e| e.to_string())?;
        if root.parent() != Some(program_files.as_path()) {
            return Err("not directly under Program Files".into());
        }
        if fs::read_dir(current).map_err(|e| e.to_string())?.count() != 5 {
            return Err("not exactly five PE files in current/".into());
        }
        for filename in REQUIRED {
            let file = current.join(filename);
            validate(&file, false)?;
            let mut f = File::open(&file).map_err(|e| e.to_string())?;
            let mut mz = [0u8;2];
            f.read_exact(&mut mz).map_err(|e| e.to_string())?;
            if &mz != b"MZ" || f.metadata().map_err(|e| e.to_string())?.len() < 512 {
                return Err("current/ contains non-PE bytes".into());
            }
        }
        Ok(app)
    }
    pub(crate) fn guarded_pr() -> Result<(), String> {
        for (key, expected) in [
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("RUNNER_OS", "Windows"),
            ("RUNNER_ENVIRONMENT", "github-hosted"),
        ] {
            if !matches!(std::env::var(key), Ok(got) if got == expected) {
                return Err(format!("not a disposable hosted Windows PR: {key}"));
            }
        }
        Ok(())
    }
    fn nonce() -> Result<String,String> {
        let mut bytes = [0u8;16];
        if unsafe { BCryptGenRandom(null_mut(),bytes.as_mut_ptr(),16,2) } != 0 {
            return Err("OS CSPRNG failed".into());
        }
        Ok(bytes.iter().map(|x| format!("{x:02x}")).collect())
    }
    fn pipe() -> Result<(Handle,Handle),String> {
        let mut attrs=SecurityAttributes {
            len:size_of::<SecurityAttributes>() as u32,
            descriptor:null_mut(), inherit:1,
        };
        let mut r=null_mut();
        let mut w=null_mut();
        if unsafe {CreatePipe(&mut r,&mut w,&mut attrs,0)} == 0 {
            return Err("CreatePipe failed".into());
        }
        let (reader,writer)=(Handle(r),Handle(w));
        if unsafe {SetHandleInformation(reader.0,HANDLE_FLAG_INHERIT,0)} == 0 {
            return Err("read-HANDLE still inheritable".into());
        }
        Ok((reader,writer))
    }
    fn environment(writer: usize, random: &str, digest: &str) -> Vec<u16> {
        let mut vars:Vec<(String,String)> = std::env::vars_os().filter_map(|(k,v)| {
            let name=k.to_string_lossy().into_owned();
            if name.to_ascii_uppercase().starts_with("AKULA_VIDEO18C_") {
                None
            } else {
                Some((name,v.to_string_lossy().into_owned()))
            }
        }).collect();
        vars.extend([
            ("AKULA_VIDEO18C_ACK_WRITE_HANDLE".into(), writer.to_string()),
            ("AKULA_VIDEO18C_ACK_NONCE".into(), random.to_string()),
            ("AKULA_VIDEO18C_GENERATION_SHA256".into(), digest.to_string()),
            ("AKULA_VIDEO18C_GENERATION_VERSION".into(), VERSION.into()),
            ("AKULA_VIDEO18C_CONTROLLER_PID".into(),
             unsafe {GetCurrentProcessId()}.to_string()),
        ]);
        vars.sort_by(|a,b|a.0.to_ascii_uppercase().cmp(&b.0.to_ascii_uppercase()));
        let mut block=Vec::<u16>::new();
        for (k,v) in vars {
            block.extend(format!("{k}={v}").encode_utf16());
            block.push(0);
        }
        block.push(0);
        block
    }
    fn creation_time(handle:*mut c_void) -> Result<FileTime,String> {
        let mut a=FileTime{low:0,high:0};
        let mut b=FileTime{low:0,high:0};
        let mut c=FileTime{low:0,high:0};
        let mut d=FileTime{low:0,high:0};
        if unsafe {GetProcessTimes(handle,&mut a,&mut b,&mut c,&mut d)} == 0 {
            return Err("could not read OS process creation time".into());
        }
        if a.low==0 && a.high==0 {return Err("unattributed OS process".into());}
        Ok(a)
    }

    // Optional second verifier can enforce SignedFive manifest authentication
    // without linking the existing standalone rustc transport test to Cargo.
    pub(crate) fn case(
        app: &Path, digest: &str, mode: &str,
        signed_check: Option<&dyn Fn(&[u8], &str, u32) -> bool>,
    ) -> Result<bool,String> {
        let root=fs::canonicalize(std::env::var_os("RUNNER_TEMP")
            .ok_or("missing RUNNER_TEMP")?).map_err(|e|e.to_string())?;
        let tag=app.parent().unwrap().parent().unwrap().file_name().unwrap()
            .to_string_lossy();
        // Two independently invoked labs share the same protected installation,
        // but must NEVER share a GUI vault/data profile. Keep fail-closed
        // no-reuse checks *within* each case, including negative trials.
        if !matches!(mode, "good" | "wrong-nonce" | "wrong-generation") {
            return Err("unknown real GUI ACK trial mode".into());
        }
        let prefix = if signed_check.is_some() {
            "Video18c02b02bSignedFrame"
        } else {
            "Video18c02bFrame"
        };
        let folder=root.join(format!("{prefix}-{tag}-{mode}"));
        if folder.exists() {return Err("reused laboratory profile".into());}
        fs::create_dir(&folder).map_err(|e|e.to_string())?;
        let vault=folder.join("vault");
        let data=folder.join("data");
        fs::create_dir(&vault).map_err(|e|e.to_string())?;
        fs::create_dir(&data).map_err(|e|e.to_string())?;
        let note=vault.join("VIDEO18C02b First Frame.md");
        fs::write(&note,b"# VIDEO18C02b First Frame\n\nNon-release GUI ACK witness.\n")
            .map_err(|e|e.to_string())?;
        let (reader,writer)=pipe()?;
        let random=nonce()?;
        if !lower_hex(&random,32) {return Err("invalid random challenge".into());}
        let mut env=environment(writer.0 as usize,&random,digest);
        let app_wide=wide(app.as_os_str());
        let current=app.parent().unwrap();
        let cwd=wide(current.as_os_str());
        let command=format!("\"{}\" --vault \"{}\" --data-dir \"{}\" --open \"{}\" --focus",
            app.display(),vault.display(),data.display(),note.display());
        let mut command_wide=wide(OsStr::new(&command));
        let mut required_bytes=0usize;
        unsafe {InitializeProcThreadAttributeList(null_mut(),1,0,&mut required_bytes);}
        if required_bytes==0 {return Err("missing STARTUPINFOEX bytes".into());}
        let mut storage=vec![0usize;required_bytes.div_ceil(size_of::<usize>())];
        let list=storage.as_mut_ptr().cast::<c_void>();
        if unsafe {InitializeProcThreadAttributeList(list,1,0,&mut required_bytes)}==0 {
            return Err("could not initialize HANDLE allowlist".into());
        }
        let mut one=[writer.0];
        if unsafe {
            UpdateProcThreadAttribute(list,0,PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                one.as_mut_ptr().cast(),size_of::<*mut c_void>(),null_mut(),null_mut())
        } == 0 {
            unsafe {DeleteProcThreadAttributeList(list);}
            return Err("not an exclusive inherited ACK HANDLE".into());
        }
        let mut startup:StartupInfoExW=unsafe{zeroed()};
        startup.startup.cb=size_of::<StartupInfoExW>() as u32;
        startup.attributes=list;
        let mut pi:ProcessInformation=unsafe{zeroed()};
        let launched=unsafe {
            CreateProcessW(app_wide.as_ptr(),command_wide.as_mut_ptr(),
                null_mut(),null_mut(),1,
                EXTENDED_STARTUPINFO_PRESENT|CREATE_UNICODE_ENVIRONMENT,
                env.as_mut_ptr().cast(),cwd.as_ptr(),&mut startup.startup,&mut pi)
        };
        unsafe {DeleteProcThreadAttributeList(list);}
        if launched==0 {return Err("CreateProcessW of real current/Akula failed".into());}
        let child=Child(Handle(pi.process));
        let _thread=Handle(pi.thread);
        drop(writer);
        if pi.pid<=1 || pi.pid==unsafe{GetCurrentProcessId()}
            || unsafe {GetProcessId(child.0.0)} != pi.pid
        {return Err("GUI child PID does not match pinned OS handle".into());}
        let first_creation=creation_time(child.0.0)?;
        let deadline=Instant::now()+FRAME_DEADLINE;
        let mut received:Option<Vec<u8>>=None;
        let mut visible:Option<*mut c_void>=None;
        while Instant::now()<deadline {
            let mut code=u32::MAX;
            if unsafe {GetExitCodeProcess(child.0.0,&mut code)}==0 || code!=STILL_ACTIVE {
                return Err("GUI child exited before a valid first-frame ACK".into());
            }
            if visible.is_none() {visible=visible_window(pi.pid);}
            let mut available=0u32;
            if unsafe {PeekNamedPipe(reader.0,null_mut(),0,null_mut(),&mut available,null_mut())}==0 {
                return Err("GUI ACK pipe failed before confirmation".into());
            }
            if available as usize>MAX_RECEIPT ||
                received.as_ref().is_some_and(|buf|buf.len()+available as usize>MAX_RECEIPT) {
                return Err("duplicate or oversized GUI ACK".into());
            }
            if available>0 {
                let mut buf=[0u8;MAX_RECEIPT];
                let mut got=0u32;
                if unsafe {ReadFile(reader.0,buf.as_mut_ptr().cast(),
                    available,&mut got,null_mut())}==0 || got==0 {
                    return Err("GUI ACK read failed".into());
                }
                if received.is_some() {return Err("repeated first-frame ACK".into());}
                received=Some(buf[..got as usize].to_vec());
            }
            if visible.is_some() && received.is_some() {break;}
            std::thread::sleep(Duration::from_millis(40));
        }
        let window=visible.ok_or("no visible Akula GUI window owned by the pinned child")?;
        let receipt=received.ok_or("no first-presented-frame ACK before bounded deadline")?;
        let expected_nonce=if mode=="wrong-nonce" {
            if random=="00000000000000000000000000000000" {
                "11111111111111111111111111111111"
            } else {
                "00000000000000000000000000000000"
            }
        } else {&random};
        let expected=format!("{HEADER}\t{expected_nonce}\t{VERSION}\t{digest}\t{}\n",pi.pid);
        // The exact bytes are compared with external state, not with child
        // asserted identity alone. A wrong-nonce trial must fail closed.
        let valid=receipt==expected.as_bytes() && creation_time(child.0.0)?==first_creation;
        // The authenticated signer must independently agree that this exact
        // receipt names its verified five-PE manifest, not merely a GUI hash.
        let signed_bound = signed_check.map_or(true, |check|
            check(&receipt, &random, pi.pid));
        std::thread::sleep(Duration::from_millis(250));
        let mut more=0u32;
        if unsafe {PeekNamedPipe(reader.0,null_mut(),0,null_mut(),&mut more,null_mut())}==0
            || more!=0 {return Err("late or repeated GUI ACK".into());}
        if unsafe {PostMessageW(window,WM_CLOSE,0,0)}==0 {
            return Err("could not close isolated GUI window".into());
        }
        if unsafe {WaitForSingleObject(child.0.0,20_000)}!=WAIT_OBJECT_0 {
            return Err("Akula GUI did not close cleanly".into());
        }
        Ok(valid && signed_bound)
    }

    pub fn main() {
        let result=(|| -> Result<(),String> {
            guarded_pr()?;
            let args:Vec<String>=std::env::args().collect();
            if args.len()!=3 || !lower_hex(&args[2],64) {
                return Err("expected protected current/akula-app.exe and verified lab SHA256".into());
            }
            let app=validate_app(&args[1])?;
            let good=case(&app,&args[2],"good",None)?;
            if !good {return Err("actual GUI failed nonce/PID/visible-window receipt".into());}
            let denied=case(&app,&args[2],"wrong-nonce",None)?;
            if denied {return Err("malformed ACK accepted as a real frame".into());}
            println!(
                "VIDEO18C02B_REAL_GUI_ACK_PASS current_gui=1 visible_os_window=1 \
                 os_child_handle=1 creation_identity=1 private_handle=1 \
                 exact_first_frame_receipt=1 wrong_nonce_denied=1 \
                 signed_generation=0 confirm=0 rollback=0 release=0"
            );
            Ok(())
        })();
        if let Err(e)=result {
            eprintln!("VIDEO18C02B_HOLD: {e}");
            std::process::exit(1);
        }
    }
}
#[cfg(windows)]
fn main(){lab::main();}
