//! VIDEO-18b-18c-01: NON-RELEASE Windows first-presented-GUI receipt sender.
//!
//! Only the existing Host::redraw path can call this after output.present()
//! and reveal() with a NONEMPTY application damage frame. A launch, decoded
//! media frame, empty redraw, or a hidden window cannot call this hook.
//!
//! An inherited, private anonymous pipe is supplied by a future trusted,
//! separate updater controller. This module does not create a listener, read
//! publisher keys, confirm an update, change an installer, or delete backups.
//! The parent MUST independently pin the launched child OS process handle,
//! exact signed five-PE manifest, protected install ACL, transaction lock and
//! a bounded read deadline. Child-controlled environment values alone are
//! NOT proof of authentic process identity or a valid update.
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};

const HEADER: &str = "AKULA-WIN-FIRST-PRESENTED-V1";
const HANDLE_ENV: &str = "AKULA_VIDEO18C_ACK_WRITE_HANDLE";
const NONCE_ENV: &str = "AKULA_VIDEO18C_ACK_NONCE";
const DIGEST_ENV: &str = "AKULA_VIDEO18C_GENERATION_SHA256";
const VERSION_ENV: &str = "AKULA_VIDEO18C_GENERATION_VERSION";
const PARENT_ENV: &str = "AKULA_VIDEO18C_CONTROLLER_PID";
const FILE_TYPE_PIPE: u32 = 3;
const HANDLE_FLAG_INHERIT: u32 = 1;
static FIRST: AtomicBool = AtomicBool::new(false);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetFileType(handle: *mut c_void) -> u32;
    fn GetHandleInformation(handle: *mut c_void, flags: *mut u32) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn WriteFile(
        handle: *mut c_void, bytes: *const c_void, size: u32,
        written: *mut u32, overlapped: *mut c_void,
    ) -> i32;
}

fn hold(reason: &str) -> String {
    format!("VIDEO18C_FIRST_GUI_ACK_HOLD: {reason}")
}

fn lower_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
}

fn semver(raw: &str) -> bool {
    let parts: Vec<_> = raw.split('.').collect();
    parts.len() == 3 && raw.len() <= 32 && parts.into_iter().all(|part| {
        !part.is_empty() && part.len() <= 10
            && (part.len() == 1 || !part.starts_with('0'))
            && part.bytes().all(|x| x.is_ascii_digit())
            && part.parse::<u32>().is_ok()
    })
}

#[derive(Debug, Clone)]
struct Challenge {
    handle: usize,
    nonce: String,
    digest: String,
    version: String,
    supervisor_pid: u32,
}

impl Challenge {
    fn parse(handle: &str, nonce: &str, digest: &str, version: &str, parent: &str)
        -> Result<Self, String>
    {
        if handle.is_empty() || handle.len() > 20 || !handle.bytes().all(|b| b.is_ascii_digit())
            || parent.is_empty() || parent.len() > 10
            || !parent.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(hold("invalid inherited handle or controller PID syntax"));
        }
        let handle = handle.parse::<usize>().map_err(|_| hold("handle overflow"))?;
        let supervisor_pid = parent.parse::<u32>().map_err(|_| hold("PID overflow"))?;
        if handle <= 16 || handle == usize::MAX || supervisor_pid <= 1
            || supervisor_pid == unsafe { GetCurrentProcessId() }
            || !lower_hex(nonce, 32) || !lower_hex(digest, 64) || !semver(version)
        {
            return Err(hold("ambiguous generation, version, pipe or controller identity"));
        }
        Ok(Self { handle, nonce: nonce.into(), digest: digest.into(),
            version: version.into(), supervisor_pid })
    }

    fn from_environment() -> Result<Option<Self>, String> {
        let fields = [HANDLE_ENV, NONCE_ENV, DIGEST_ENV, VERSION_ENV, PARENT_ENV]
            .map(std::env::var);
        if fields.iter().all(Result::is_err) { return Ok(None); }
        let mut iter=fields.into_iter();
        let handle=iter.next().unwrap().map_err(|_| hold("partial ACK challenge"))?;
        let nonce=iter.next().unwrap().map_err(|_| hold("partial ACK challenge"))?;
        let digest=iter.next().unwrap().map_err(|_| hold("partial ACK challenge"))?;
        let version=iter.next().unwrap().map_err(|_| hold("partial ACK challenge"))?;
        let parent=iter.next().unwrap().map_err(|_| hold("partial ACK challenge"))?;
        Self::parse(&handle,&nonce,&digest,&version,&parent).map(Some)
    }
}

// Only writes a small bounded receipt to an inherited pipe. The controller
// MUST NOT accept this alone: it must compare actual Child PID and file bytes
// under the OS transaction lock before calling any confirm operation.
fn emit(challenge: &Challenge) -> Result<(), String> {
    let pipe=challenge.handle as *mut c_void;
    if unsafe { GetFileType(pipe) } != FILE_TYPE_PIPE {
        return Err(hold("ACK target is not an actual Windows pipe"));
    }
    let mut flags=0u32;
    if unsafe { GetHandleInformation(pipe,&mut flags) } == 0
        || flags & HANDLE_FLAG_INHERIT == 0
    {
        return Err(hold("ACK pipe is not an inherited handle"));
    }
    let child_pid=unsafe { GetCurrentProcessId() };
    if child_pid == challenge.supervisor_pid {
        return Err(hold("supervisor and GUI PID must be distinct"));
    }
    let body=format!(
        "{HEADER}\t{}\t{}\t{}\t{}\n",
        challenge.nonce,challenge.version,challenge.digest,child_pid
    );
    if body.len() > 256 { return Err(hold("oversized ACK packet")); }
    let mut sent=0u32;
    if unsafe { WriteFile(pipe,body.as_ptr().cast(),body.len() as u32,&mut sent,
        std::ptr::null_mut()) } == 0 || sent as usize != body.len()
    {
        return Err(hold("pipe ACK write failed or incomplete"));
    }
    Ok(())
}

/// Source of this signal is Host::redraw after a real nonempty output.present
/// and reveal, never the media decoding thread or a successful process launch.
/// Keep the UI thread nonblocking; exactly one send attempt per process.
pub(crate) fn on_first_presented_frame() {
    if FIRST.swap(true, Ordering::AcqRel) { return; }
    std::thread::spawn(|| match Challenge::from_environment() {
        Ok(None) => {}
        Ok(Some(challenge)) => {
            if let Err(error)=emit(&challenge) { eprintln!("{error}"); }
        }
        Err(error) => eprintln!("{error}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreatePipe(read: *mut *mut c_void,write: *mut *mut c_void,
            attributes: *const c_void,size: u32) -> i32;
        fn SetHandleInformation(handle: *mut c_void, mask: u32, flags: u32) -> i32;
        fn ReadFile(handle: *mut c_void,bytes: *mut c_void,size: u32,
            read: *mut u32,overlapped: *mut c_void) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn partial_wrong_identity_and_noncanonical_version_refuse() {
        for field in ["", "x", "-1", "0", "10", "184467440737095516160000"] {
            assert!(Challenge::parse(field,NONCE,DIGEST,"0.3.21","9999").is_err());
        }
        for version in ["0.3.021","0.3.21-beta","v0.3.21","4294967296.1.1"] {
            assert!(Challenge::parse("256",NONCE,DIGEST,version,"9999").is_err());
        }
        assert!(Challenge::parse("256","invalid",DIGEST,"0.3.21","9999").is_err());
        assert!(Challenge::parse("256",NONCE,&"A".repeat(64),"0.3.21","9999").is_err());
        assert!(Challenge::parse("256",NONCE,DIGEST,"0.3.21","1").is_err());
        assert!(Challenge::parse("256",NONCE,DIGEST,"0.3.21",
            &unsafe { GetCurrentProcessId() }.to_string()).is_err());
    }

    struct Pair(*mut c_void,*mut c_void);
    impl Pair {
        fn new() -> Self {
            let (mut read,mut write)=(ptr::null_mut(),ptr::null_mut());
            assert_ne!(unsafe {CreatePipe(&mut read,&mut write,ptr::null(),0)},0);
            Pair(read,write)
        }
    }
    impl Drop for Pair {
        fn drop(&mut self) {
            unsafe {CloseHandle(self.0);CloseHandle(self.1);}
        }
    }

    #[test]
    fn noninherited_pipe_and_regular_windows_handle_cannot_ack() {
        let pair=Pair::new();
        let invalid=Challenge::parse(&(pair.1 as usize).to_string(),
            NONCE,DIGEST,"0.3.21","9999").unwrap();
        assert!(emit(&invalid).unwrap_err().contains("not an inherited"));
        assert_ne!(unsafe { SetHandleInformation(pair.1,HANDLE_FLAG_INHERIT,HANDLE_FLAG_INHERIT) },0);
        emit(&invalid).unwrap();
        let mut data=[0u8;256];
        let mut count=0u32;
        assert_ne!(unsafe { ReadFile(pair.0,data.as_mut_ptr().cast(),256,&mut count,
            ptr::null_mut()) },0);
        let line=std::str::from_utf8(&data[..count as usize]).unwrap();
        assert_eq!(line,format!("{HEADER}\t{NONCE}\t0.3.21\t{DIGEST}\t{}\n",
            unsafe { GetCurrentProcessId() }));
        // A borrowed test pipe is closed by Drop, not by the app emitter.
    }
}
