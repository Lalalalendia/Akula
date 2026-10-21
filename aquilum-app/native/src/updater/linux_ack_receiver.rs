//! RELEASE-LINUX-ACK-PASSCRED-01: one-use supervisor-side GUI witness.
//!
//! The GUI emits an ACK only after its real first nonempty presented frame.
//! This module verifies the receiving side: Linux SCM_CREDENTIALS reports the
//! ACTUAL sender PID/UID/GID rather than trusting a PID written by that sender.
//! A pidfd holds sender identity while procfs ancestry/session are validated.
//!
//! This is NOT publisher authority or installed-update permission. A future
//! updater supervisor must additionally bind a signed candidate to the exact
//! executable inode and a durable anti-rollback witness before using an ACK.

#![cfg(target_os = "linux")]

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::raw::c_long;
use std::os::unix::net::UnixStream;
use std::process::Child;
use std::time::Duration;

const HEADER: &str = "AKULA-LINUX-FIRST-GUI-V1";
const ACK_LIMIT: usize = 256;
const SOL_SOCKET: i32 = 1;
const SO_PASSCRED: i32 = 16;
const SCM_CREDENTIALS: i32 = 2;
const MSG_CTRUNC: i32 = 0x08;
const MSG_TRUNC: i32 = 0x20;
const SYS_PIDFD_OPEN: c_long = 434;

#[repr(C)]
struct Iovec {
    base: *mut c_void,
    len: usize,
}

#[repr(C)]
struct MsgHdr {
    name: *mut c_void,
    name_len: u32,
    iov: *mut Iovec,
    iov_len: usize,
    control: *mut c_void,
    control_len: usize,
    flags: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct Cmsghdr {
    len: usize,
    level: i32,
    kind: i32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Ucred {
    pid: i32,
    uid: u32,
    gid: u32,
}

unsafe extern "C" {
    fn setsockopt(fd: i32, level: i32, option: i32, value: *const c_void, len: u32) -> i32;
    fn recvmsg(fd: i32, message: *mut MsgHdr, flags: i32) -> isize;
    fn getuid() -> u32;
    fn getgid() -> u32;
    fn getpgid(pid: i32) -> i32;
    fn syscall(number: c_long, ...) -> c_long;
}

fn hold(why: &str) -> String {
    format!("AKULA_LINUX_ACK_RECEIVER_HOLD: {why}")
}

fn hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn align(length: usize) -> usize {
    let word = core::mem::size_of::<usize>();
    (length + word - 1) & !(word - 1)
}

fn random_nonce() -> Result<String, String> {
    let mut random = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut random))
        .map_err(|e| format!("{}: {e}", hold("OS nonce source unavailable")))?;
    Ok(random.iter().map(|c| format!("{c:02x}")).collect())
}

fn enable_kernel_sender_credentials(socket: &UnixStream) -> Result<(), String> {
    let one: i32 = 1;
    let status = unsafe {
        setsockopt(
            socket.as_raw_fd(), SOL_SOCKET, SO_PASSCRED,
            (&raw const one).cast(), core::mem::size_of_val(&one) as u32,
        )
    };
    if status != 0 {
        return Err(format!("{}: {}", hold("SO_PASSCRED failed"), io::Error::last_os_error()));
    }
    Ok(())
}

fn read_one_kernel_attributed_write(socket: &UnixStream) -> Result<(String, Ucred), String> {
    let mut buffer = [0u8; ACK_LIMIT + 1];
    // Alignment is critical: cmsghdr contains a native usize.
    let mut ancillary = [0usize; 8];
    let mut vector = Iovec { base: buffer.as_mut_ptr().cast(), len: buffer.len() };
    let mut header = MsgHdr {
        name: core::ptr::null_mut(),
        name_len: 0,
        iov: &raw mut vector,
        iov_len: 1,
        control: ancillary.as_mut_ptr().cast(),
        control_len: core::mem::size_of_val(&ancillary),
        flags: 0,
    };
    let got = unsafe { recvmsg(socket.as_raw_fd(), &raw mut header, 0) };
    if got <= 0 {
        return Err(format!("{}: {}", hold("no GUI ACK before EOF or timeout"), io::Error::last_os_error()));
    }
    if got as usize > ACK_LIMIT || (header.flags & (MSG_CTRUNC | MSG_TRUNC)) != 0 {
        return Err(hold("oversized ACK or truncated kernel sender credentials"));
    }

    let mut found = None;
    let control_base = ancillary.as_ptr().cast::<u8>();
    let control_end = header.control_len;
    let mut at = 0usize;
    while at + core::mem::size_of::<Cmsghdr>() <= control_end {
        let header_ptr = unsafe { control_base.add(at).cast::<Cmsghdr>() };
        let entry = unsafe { core::ptr::read_unaligned(header_ptr) };
        let minimum = align(core::mem::size_of::<Cmsghdr>());
        if entry.len < minimum || entry.len > control_end - at {
            return Err(hold("malformed ancillary credentials"));
        }
        if entry.level == SOL_SOCKET && entry.kind == SCM_CREDENTIALS {
            if found.is_some() || entry.len < minimum + core::mem::size_of::<Ucred>() {
                return Err(hold("missing, duplicate or truncated SCM_CREDENTIALS"));
            }
            let ptr = unsafe { control_base.add(at + minimum).cast::<Ucred>() };
            found = Some(unsafe { core::ptr::read_unaligned(ptr) });
        }
        at += align(entry.len);
    }
    let credentials = found.ok_or_else(|| hold("ACK sender kernel credentials were absent"))?;
    let line = std::str::from_utf8(&buffer[..got as usize])
        .map_err(|_| hold("ACK is not ASCII-compatible UTF-8"))?
        .to_owned();
    Ok((line, credentials))
}

fn parse_exact_wire(line: &str, nonce: &str, digest: &str) -> Result<i32, String> {
    if line.len() > ACK_LIMIT
        || !line.ends_with('\n')
        || line.bytes().filter(|c| *c == b'\n').count() != 1
        || line.contains('\r')
    {
        return Err(hold("incomplete, oversized or repeated ACK wire message"));
    }
    let parts: Vec<_> = line.trim_end_matches('\n').split('\t').collect();
    if parts.len() != 4 || parts[0] != HEADER || parts[1] != nonce || parts[3] != digest {
        return Err(hold("protocol, nonce or signed-package-digest mismatch"));
    }
    let pid = parts[2].parse::<i32>().map_err(|_| hold("malformed GUI PID"))?;
    if pid <= 1 || parts[2] != pid.to_string() {
        return Err(hold("noncanonical GUI PID"));
    }
    Ok(pid)
}

fn pid_is_in_live_wrapper_tree(gui: i32, wrapper: i32) -> bool {
    let mut pid = gui;
    for _ in 0..12 {
        if pid == wrapper {
            return true;
        }
        if pid <= 1 {
            return false;
        }
        let Ok(value) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return false;
        };
        let Some(parent) = value.lines()
            .find_map(|line| line.strip_prefix("PPid:"))
            .and_then(|rest| rest.trim().parse::<i32>().ok()) else {
                return false;
            };
        if parent <= 1 || parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

fn pin_live_sender_pid(pid: i32) -> Result<OwnedFd, String> {
    #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
    {
        let descriptor = unsafe { syscall(SYS_PIDFD_OPEN, c_long::from(pid), 0 as c_long) };
        if descriptor < 0 || descriptor > i32::MAX as c_long {
            return Err(format!("{}: {}", hold("could not pin GUI PID with pidfd"), io::Error::last_os_error()));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(descriptor as i32) })
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = pid;
        Err(hold("pidfd syscall architecture not approved"))
    }
}

/// Witness of a single kernel-attributed, live GUI response. Does NOT confer
/// installation, publisher identity or release acceptance authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GuiAckWitness {
    pub(crate) actual_gui_pid: i32,
    pub(crate) candidate_digest: String,
}

/// The child socket is returned separately, to be passed as an inherited FD
/// only when a future supervisor has independently approved the exact file.
pub(crate) struct AckReceiver {
    socket: UnixStream,
    nonce: String,
    digest: String,
    consumed: bool,
}

impl AckReceiver {
    pub(crate) fn new(signed_digest: &str) -> Result<(Self, UnixStream), String> {
        if !hex(signed_digest, 64) {
            return Err(hold("expected package digest must be canonical lowercase SHA-256"));
        }
        let nonce = random_nonce()?;
        let (parent, child) = UnixStream::pair().map_err(|e| e.to_string())?;
        enable_kernel_sender_credentials(&parent)?;
        Ok((Self { socket: parent, nonce, digest: signed_digest.to_owned(), consumed: false }, child))
    }

    pub(crate) fn challenge_environment(&self, inherited_fd: i32) -> Result<[(&'static str, String); 4], String> {
        if !(3..=1024).contains(&inherited_fd) {
            return Err(hold("invalid inherited ACK socket number"));
        }
        Ok([
            ("AKULA_LINUX_UPDATE_ACK_FD", inherited_fd.to_string()),
            ("AKULA_LINUX_UPDATE_NONCE", self.nonce.clone()),
            ("AKULA_LINUX_UPDATE_PARENT_PID", std::process::id().to_string()),
            ("AKULA_LINUX_UPDATE_IMAGE_SHA256", self.digest.clone()),
        ])
    }

    pub(crate) fn receive_once(&mut self, launched_wrapper: &mut Child) -> Result<GuiAckWitness, String> {
        self.receive_with_timeout(launched_wrapper, Duration::from_secs(15))
    }

    fn receive_with_timeout(
        &mut self,
        launched_wrapper: &mut Child,
        timeout: Duration,
    ) -> Result<GuiAckWitness, String> {
        if self.consumed {
            return Err(hold("ACK challenge was already consumed"));
        }
        // Consumption is permanent even on timeout, malformed bytes or denial:
        // never retry the same generation nonce after a failed attempt.
        self.consumed = true;
        if launched_wrapper.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err(hold("launched AppImage already terminated before ACK"));
        }
        let wrapper = launched_wrapper.id() as i32;
        if wrapper <= 1 || unsafe { getpgid(wrapper) } != wrapper {
            return Err(hold("AppImage wrapper must own an isolated OS session"));
        }
        self.socket.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
        let (wire, kernel) = read_one_kernel_attributed_write(&self.socket)?;
        let claimed = parse_exact_wire(&wire, &self.nonce, &self.digest)?;
        if claimed != kernel.pid {
            return Err(hold("wire PID differs from Linux SCM_CREDENTIALS sender"));
        }
        if kernel.uid != unsafe { getuid() } || kernel.gid != unsafe { getgid() } {
            return Err(hold("GUI sender UID/GID differs from launched supervisor"));
        }
        // Hold pidfd across all process-tree checks to prevent PID recycling.
        let _pinned_sender = pin_live_sender_pid(kernel.pid)?;
        if unsafe { getpgid(kernel.pid) } != wrapper
            || !pid_is_in_live_wrapper_tree(kernel.pid, wrapper)
        {
            return Err(hold("sender does not belong to the launched AppImage process tree"));
        }
        if launched_wrapper.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err(hold("AppImage wrapper exited during ACK inspection"));
        }
        Ok(GuiAckWitness { actual_gui_pid: kernel.pid, candidate_digest: self.digest.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CHILD_FD: i32 = 198;
    unsafe extern "C" {
        fn dup2(from: i32, to: i32) -> i32;
        fn setsid() -> i32;
    }

    #[test]
    fn ack_child_helper() {
        let Ok(phase) = std::env::var("AKULA_ACK_RECEIVER_TEST_CHILD") else { return };
        let fd: i32 = std::env::var("AKULA_LINUX_UPDATE_ACK_FD").unwrap().parse().unwrap();
        let nonce = std::env::var("AKULA_LINUX_UPDATE_NONCE").unwrap();
        let hash = std::env::var("AKULA_LINUX_UPDATE_IMAGE_SHA256").unwrap();
        let mut sender = unsafe { UnixStream::from_raw_fd(fd) };
        if phase != "silent" {
            let pid = std::process::id();
            let body = match phase.as_str() {
                "good" => format!("{HEADER}\t{nonce}\t{pid}\t{hash}\n"),
                "bad-nonce" => format!("{HEADER}\t{}\t{pid}\t{hash}\n", "0".repeat(32)),
                "bad-hash" => format!("{HEADER}\t{nonce}\t{pid}\t{}\n", "0".repeat(64)),
                "bad-pid" => format!("{HEADER}\t{nonce}\t{}\t{hash}\n", pid + 1),
                "extra" => format!("{HEADER}\t{nonce}\t{pid}\t{hash}\nsecond-message\n"),
                _ => panic!("unknown test child phase"),
            };
            sender.write_all(body.as_bytes()).unwrap();
            sender.shutdown(std::net::Shutdown::Write).unwrap();
        }
        std::thread::sleep(Duration::from_millis(950));
    }

    fn registered_helper() -> String {
        let listing = Command::new(std::env::current_exe().unwrap())
            .arg("--list").output().unwrap();
        assert!(listing.status.success());
        let list = String::from_utf8_lossy(&listing.stdout);
        let suite = module_path!().split_once("::")
            .map_or(module_path!(), |(_, rest)| rest);
        let exact = format!("{suite}::ack_child_helper");
        assert!(list.lines().any(|line| line == format!("{exact}: test")));
        exact
    }

    fn launch(
        receiver: &AckReceiver,
        socket: &UnixStream,
        phase: &str,
    ) -> Child {
        let source_fd = socket.as_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg(registered_helper())
            .arg("--nocapture")
            .envs(receiver.challenge_environment(CHILD_FD).unwrap())
            .env("AKULA_ACK_RECEIVER_TEST_CHILD", phase)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if dup2(source_fd, CHILD_FD) < 0 || setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        command.spawn().unwrap()
    }

    fn stop(child: &mut Child) {
        let _ = child.kill();
        child.wait().unwrap();
    }

    #[test]
    fn os_random_nonce_and_invalid_digest_refusal() {
        let (a, _) = AckReceiver::new(DIGEST).unwrap();
        let (b, _) = AckReceiver::new(DIGEST).unwrap();
        assert!(hex(&a.nonce, 32));
        assert_ne!(a.nonce, b.nonce, "OS challenges cannot be fixed test tokens");
        assert!(AckReceiver::new("AAAA").is_err());
    }

    #[test]
    fn actual_kernel_credentials_accept_only_once() {
        let (mut receiver, socket) = AckReceiver::new(DIGEST).unwrap();
        let mut child = launch(&receiver, &socket, "good");
        drop(socket);
        let evidence = receiver.receive_once(&mut child).unwrap();
        assert_eq!(evidence.actual_gui_pid, child.id() as i32);
        assert_eq!(evidence.candidate_digest, DIGEST);
        assert!(receiver.receive_once(&mut child).unwrap_err().contains("already consumed"));
        stop(&mut child);
    }

    #[test]
    fn wrong_nonce_hash_pid_and_repeated_wire_are_rejected() {
        for phase in ["bad-nonce", "bad-hash", "bad-pid", "extra"] {
            let (mut receiver, socket) = AckReceiver::new(DIGEST).unwrap();
            let mut child = launch(&receiver, &socket, phase);
            drop(socket);
            assert!(receiver.receive_with_timeout(&mut child, Duration::from_secs(2)).is_err(), "{phase}");
            stop(&mut child);
        }
    }

    #[test]
    fn missing_ack_consumes_nonce_without_accepting_late_replay() {
        let (mut receiver, socket) = AckReceiver::new(DIGEST).unwrap();
        let mut child = launch(&receiver, &socket, "silent");
        drop(socket);
        assert!(receiver.receive_with_timeout(&mut child, Duration::from_millis(250)).is_err());
        assert!(receiver.receive_once(&mut child).unwrap_err().contains("already consumed"));
        stop(&mut child);
    }

    #[test]
    fn unrelated_session_cannot_claim_same_generation_even_with_leaked_fd() {
        let (mut receiver, socket) = AckReceiver::new(DIGEST).unwrap();
        let mut true_wrapper = launch(&receiver, &socket, "silent");
        let mut impostor = launch(&receiver, &socket, "good");
        drop(socket);
        // The kernel correctly attributes the message to the impostor,
        // but it has a different OS session and is NOT a descendant of
        // the independently launched, still-live expected wrapper.
        let error = receiver.receive_with_timeout(&mut true_wrapper, Duration::from_secs(2)).unwrap_err();
        assert!(error.contains("process tree"), "{error}");
        stop(&mut impostor);
        stop(&mut true_wrapper);
    }
}
