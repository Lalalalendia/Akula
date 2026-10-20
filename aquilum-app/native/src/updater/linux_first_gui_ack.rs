//! RELEASE-LINUX-FIRST-GUI-ACK-01 — a non-authorizing first-frame signal.
//!
//! The real installed GUI emits a one-time receipt only after the first
//! nonempty presented frame and native-window reveal. Delivery uses an
//! inherited Unix stream FD supplied by a future separately trusted updater
//! supervisor. This does NOT grant installation, establish publisher trust,
//! attest AppImage bytes, or permit removal of the previous version.
//!
//! Linux-only; no disk writes and no network listener.

use std::io::{self, Write};
use std::os::fd::{BorrowedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const HEADER: &str = "AKULA-LINUX-FIRST-GUI-V1";
const FD_ENV: &str = "AKULA_LINUX_UPDATE_ACK_FD";
const NONCE_ENV: &str = "AKULA_LINUX_UPDATE_NONCE";
const PARENT_ENV: &str = "AKULA_LINUX_UPDATE_PARENT_PID";
const DIGEST_ENV: &str = "AKULA_LINUX_UPDATE_IMAGE_SHA256";
static FIRST_PRESENTED: AtomicBool = AtomicBool::new(false);

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PeerCred {
    pid: i32,
    uid: u32,
    gid: u32,
}

unsafe extern "C" {
    fn getpid() -> i32;
    fn getppid() -> i32;
    fn getuid() -> u32;
    fn getgid() -> u32;
    fn getsockopt(
        fd: i32,
        level: i32,
        option_name: i32,
        option_value: *mut core::ffi::c_void,
        option_length: *mut u32,
    ) -> i32;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Challenge {
    fd: RawFd,
    nonce: String,
    parent_pid: i32,
    image_sha256: String,
}

fn refusal(message: &str) -> String {
    format!("AKULA_LINUX_GUI_ACK_HOLD: {message}")
}

fn lower_hex(s: &str, n: usize) -> bool {
    s.len() == n &&
        s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Challenge {
    fn parse(fd: &str, parent_pid: &str, nonce: &str, digest: &str) -> Result<Self, String> {
        let fd = fd.parse::<RawFd>().map_err(|_| refusal("invalid inherited descriptor"))?;
        let parent_pid = parent_pid.parse::<i32>()
            .map_err(|_| refusal("invalid expected supervisor PID"))?;
        if !(3..=1024).contains(&fd) || parent_pid <= 1 {
            return Err(refusal("invalid descriptor or supervisor identity"));
        }
        if !lower_hex(nonce, 32) || !lower_hex(digest, 64) {
            return Err(refusal("invalid generation nonce or package digest"));
        }
        Ok(Self {
            fd, parent_pid, nonce: nonce.to_owned(),
            image_sha256: digest.to_owned(),
        })
    }

    fn from_environment() -> Result<Option<Self>, String> {
        let values = [FD_ENV, PARENT_ENV, NONCE_ENV, DIGEST_ENV]
            .map(std::env::var);
        if values.iter().all(Result::is_err) {
            return Ok(None);
        }
        let mut values = values.into_iter();
        let fd = values.next().unwrap().map_err(|_| refusal("partial ACK challenge"))?;
        let parent = values.next().unwrap().map_err(|_| refusal("partial ACK challenge"))?;
        let nonce = values.next().unwrap().map_err(|_| refusal("partial ACK challenge"))?;
        let sha = values.next().unwrap().map_err(|_| refusal("partial ACK challenge"))?;
        Ok(Some(Self::parse(&fd, &parent, &nonce, &sha)?))
    }
}

fn peer_identity(fd: RawFd) -> Result<(i32, u32, u32), String> {
    // Linux SOL_SOCKET=1, SO_TYPE=3, SOCK_STREAM=1, SO_PEERCRED=17.
    // SO_PEERCRED on a socketpair describes its creator. A future supervising
    // process must still bind the ACK to its own Child PID, signed candidate,
    // and generation; this observation is not a standalone authenticator.
    let mut kind: i32 = 0;
    let mut kind_len = core::mem::size_of::<i32>() as u32;
    let kind_rc = unsafe {
        getsockopt(
            fd, 1, 3, (&raw mut kind).cast(), &raw mut kind_len,
        )
    };
    if kind_rc != 0 || kind_len as usize != core::mem::size_of::<i32>() || kind != 1 {
        return Err(refusal("inherited descriptor is not a Unix stream socket"));
    }
    let mut cred = PeerCred::default();
    let mut len = core::mem::size_of::<PeerCred>() as u32;
    let rc = unsafe {
        getsockopt(fd, 1, 17, (&raw mut cred).cast(), &raw mut len)
    };
    if rc != 0 || len as usize != core::mem::size_of::<PeerCred>() {
        return Err(refusal("could not establish inherited socket peer credentials"));
    }
    Ok((cred.pid, cred.uid, cred.gid))
}

// AppImage's extract-and-run runtime may fork a payload and wait for it.
// Therefore the GUI need not be the supervisor's *direct* child: require
// a short, live OS parent chain back to the trusted socket creator instead.
// A future accepting supervisor MUST separately pin the actual GUI PID with
// pidfd and check membership in the explicitly launched AppImage process tree.
fn supervisor_in_ancestry(supervisor: i32) -> bool {
    let mut pid = unsafe { getppid() };
    for _ in 0..12 {
        if pid == supervisor {
            return true;
        }
        if pid <= 1 || pid == unsafe { getpid() } {
            return false;
        }
        let Ok(raw) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return false;
        };
        // The PPid field comes from procfs, not from the child's environment.
        let Some(next) = raw.lines()
            .find_map(|line| line.strip_prefix("PPid:"))
            .and_then(|value| value.trim().parse::<i32>().ok())
        else {
            return false;
        };
        if next == pid {
            return false;
        }
        pid = next;
    }
    false
}

fn emit_after_presented_gui(challenge: &Challenge) -> Result<(), String> {
    let (peer_pid, peer_uid, peer_gid) = peer_identity(challenge.fd)?;
    if peer_pid != challenge.parent_pid
        || !supervisor_in_ancestry(challenge.parent_pid)
        || peer_uid != unsafe { getuid() }
        || peer_gid != unsafe { getgid() }
    {
        return Err(refusal("wrong OS supervisor process or socket identity"));
    }
    let process = unsafe { getpid() };
    let body = format!(
        "{HEADER}\t{}\t{}\t{}\n",
        challenge.nonce, process, challenge.image_sha256
    );
    // Borrow the inherited descriptor; cloning avoids taking ownership of a
    // caller's FD. Network sockets, files and named receipts are not opened.
    let borrowed = unsafe { BorrowedFd::borrow_raw(challenge.fd) };
    let owned = borrowed.try_clone_to_owned().map_err(|e| e.to_string())?;
    let mut socket = UnixStream::from(owned);
    socket.set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    socket.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    socket.shutdown(std::net::Shutdown::Write)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// ONLY the installed first-frame hook calls this, after output.present()
/// and native-window reveal(). No ACK is sent without an inherited channel.
/// The handler is asynchronous: the GUI thread must never block on an update.
#[allow(dead_code)]
pub(crate) fn on_first_presented_frame() {
    if FIRST_PRESENTED.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(|| match Challenge::from_environment() {
        Ok(None) => {}
        Ok(Some(challenge)) => {
            if let Err(e) = emit_after_presented_gui(&challenge) {
                eprintln!("{e}");
            }
        }
        Err(e) => eprintln!("{e}"),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::fd::AsRawFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU64, Ordering};

    const NONCE: &str = "0123456789abcdef0123456789abcdef";
    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    static RUN: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" {
        fn dup2(oldfd: i32, newfd: i32) -> i32;
    }

    #[test]
    fn parser_rejects_partial_and_ambiguous_generation() {
        assert!(Challenge::parse("x", "1", NONCE, DIGEST).is_err());
        assert!(Challenge::parse("3", "1", NONCE, DIGEST).is_err());
        assert!(Challenge::parse("3", "100", "nonce", DIGEST).is_err());
        assert!(Challenge::parse("3", "100", NONCE, "A".repeat(64).as_str()).is_err());
        assert!(Challenge::parse("0", "100", NONCE, DIGEST).is_err());
        let witness = Challenge::parse("198", "100", NONCE, DIGEST).unwrap();
        assert_eq!(witness.fd, 198);
        assert_eq!(witness.nonce, NONCE);
    }

    #[test]
    fn regular_file_is_never_an_ack_channel() {
        let unique = RUN.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "akula-ack-not-a-socket-{}-{unique}", std::process::id(),
        ));
        let file = std::fs::OpenOptions::new().create_new(true).write(true)
            .open(&path).unwrap();
        let challenge = Challenge {
            fd: file.as_raw_fd(),
            nonce: NONCE.into(),
            parent_pid: unsafe { getppid() },
            image_sha256: DIGEST.into(),
        };
        assert!(emit_after_presented_gui(&challenge).is_err());
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn subprocess_ack_helper() {
        let Ok(phase) = std::env::var("AKULA_GUI_ACK_CHILD_PHASE") else {
            return;
        };
        let challenge = Challenge::from_environment().unwrap().unwrap();
        match phase.as_str() {
            "proper" => emit_after_presented_gui(&challenge).unwrap(),
            "wrong-parent" => assert!(
                emit_after_presented_gui(&challenge).unwrap_err()
                    .contains("wrong OS supervisor process"),
            ),
            _ => panic!("unsupported test phase"),
        }
    }

    fn helper_name() -> String {
        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--list").output().unwrap();
        assert!(output.status.success());
        let listing = String::from_utf8_lossy(&output.stdout);
        let suite = module_path!().split_once("::")
            .map_or(module_path!(), |(_, rest)| rest);
        let wanted = format!("{suite}::subprocess_ack_helper");
        assert!(
            listing.lines().any(|line| line == format!("{wanted}: test")),
            "the exact Linux ACK child helper is not registered",
        );
        wanted
    }

    fn spawn_pair(phase: &str) -> (String, i32) {
        let (parent, child) = UnixStream::pair().unwrap();
        parent.set_read_timeout(Some(Duration::from_secs(8))).unwrap();
        let fd = child.as_raw_fd();
        const CHILD_FD: i32 = 198;
        let pid = std::process::id();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact").arg(helper_name()).arg("--nocapture")
            .env(FD_ENV, CHILD_FD.to_string())
            .env(PARENT_ENV, if phase == "wrong-parent" { pid + 1 } else { pid }.to_string())
            .env(NONCE_ENV, NONCE)
            .env(DIGEST_ENV, DIGEST)
            .env("AKULA_GUI_ACK_CHILD_PHASE", phase)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // dup2 in pre_exec performs no allocation or locking after fork.
        unsafe {
            command.pre_exec(move || {
                if dup2(fd, CHILD_FD) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut launched = command.spawn().unwrap();
        drop(child);
        let mut line = String::new();
        BufReader::new(parent).read_line(&mut line).unwrap();
        let status = launched.wait().unwrap();
        assert!(status.success(), "the child assertion or ACK process failed");
        (line, launched.id() as i32)
    }

    #[test]
    fn real_inherited_stream_ack_has_child_pid_nonce_and_generation_digest() {
        let (line, child_pid) = spawn_pair("proper");
        assert_eq!(line, format!("{HEADER}\t{NONCE}\t{child_pid}\t{DIGEST}\n"));
    }

    #[test]
    fn wrong_parent_cannot_emit_even_with_provided_nonce_and_fd() {
        let (line, _) = spawn_pair("wrong-parent");
        assert!(line.is_empty(), "untrusted parent spoof produced an ACK");
    }
}
