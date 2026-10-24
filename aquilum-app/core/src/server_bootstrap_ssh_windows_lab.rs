//! B0c: actual Windows OpenSSH signed-host-KEX + fixed read-only preflight.
//! Explicitly opt-in CI/laboratory feature, NOT a public install API.
//!
//! Operator provides the exact ed25519 SSH host PUBLIC key and independently
//! obtained wire SHA-256. ssh.exe (OS-protected System32 binary) verifies the
//! live server's signed KEX via StrictHostKeyChecking=yes before it can query
//! fixed, read-only Debian/Ubuntu host facts. No TOFU, ssh-keyscan, remotely
//! chosen known_hosts, passwords, SSH identity path, shell input or writes.
//! Auth uses only the user's pre-existing Windows OpenSSH ssh-agent; its private
//! key material is never read/stored by Akula. No UI currently invokes this.
#![cfg(all(windows, feature = "server-bootstrap-b0-openssh-lab"))]

use crate::server_bootstrap_lab::{
    B0Error, B0Input, MachineArch, PreviewTransport, ReadOnlyHostFacts, ServerTarget, SupportedOs,
};
use ring::{constant_time, digest};
use std::collections::BTreeMap;
use std::ffi::{c_void, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

mod scan;
#[cfg(feature = "server-root-stage-ssh-lab")]
mod root_stage;

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const READ_CONTROL: u32 = 0x0002_0000;
const FILE_SHARE_READ: u32 = 0x1;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INVALID_HANDLE: *mut c_void = -1isize as *mut c_void;
const MAX_OUTPUT: u64 = 8192;
const DEADLINE: Duration = Duration::from_secs(12);

/// Developer-only disposable loopback diagnostic. Never output an SSH path,
/// a key, an agent identity, remote stdout or the actual system error text.
fn report_local_ci(stage: &'static str, code: Option<i32>) {
    if cfg!(debug_assertions)
        && std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true")
        && std::env::var("AKULA_B0C_CI_DIAG").ok().as_deref() == Some("LOOPBACK_ONLY")
    {
        eprintln!("B0C_PIN_STAGE={stage} OS_CODE={code:?}");
    }
}
fn pin_io(stage: &'static str, error: std::io::Error) -> B0Error {
    report_local_ci(stage, error.raw_os_error());
    B0Error::KeyUnavailable
}
fn pin_refused(stage: &'static str) -> B0Error {
    report_local_ci(stage, None);
    B0Error::KeyUnavailable
}

const FACTS_COMMAND: &str = concat!(
    "LC_ALL=C; export LC_ALL; ",
    "printf 'AKULA_B0_OS='; grep -m1 '^ID=' /etc/os-release || printf 'ID=unknown\\n'; ",
    "printf 'AKULA_B0_ARCH='; uname -m; ",
    "printf 'AKULA_B0_SYSTEMD='; if test -d /run/systemd/system; then echo yes; else echo no; fi; ",
    "printf 'AKULA_B0_SUDO='; if sudo -n -l >/dev/null 2>&1; then echo yes; else echo no; fi; ",
    "printf 'AKULA_B0_CPU='; getconf _NPROCESSORS_ONLN; ",
    "printf 'AKULA_B0_RAM='; awk '/^MemTotal:/ { printf \"%d\\n\", $2 / 1024; exit }' /proc/meminfo; ",
    "printf 'AKULA_B0_DISK='; df -Pk / | awk 'NR == 2 { printf \"%d\\n\", $4 / 1024 }'"
);

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    fn CreateFileW(
        path: *const u16, access: u32, share: u32, security: *mut c_void,
        disposition: u32, flags: u32, template: *mut c_void,
    ) -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn GetFileInformationByHandleEx(
        handle: *mut c_void, class: u32,
        info: *mut c_void, info_size: u32,
    ) -> i32;
}
fn utf16_z(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}
struct HeldDirectory(*mut c_void);
impl Drop for HeldDirectory {
    fn drop(&mut self) {
        // SAFETY: this is one opened directory HANDLE, never a pseudo handle.
        unsafe { CloseHandle(self.0); }
    }
}
fn pin_directory(path: &Path) -> Result<HeldDirectory, B0Error> {
    let wide = utf16_z(path);
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(), READ_CONTROL | FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
            std::ptr::null_mut(), OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if handle.is_null() || handle == INVALID_HANDLE {
        return Err(B0Error::KeyUnavailable);
    }
    let held = HeldDirectory(handle);
    #[repr(C)]
    #[derive(Default)]
    struct AttributeTag { flags: u32, reparse_tag: u32 }
    let mut info = AttributeTag::default();
    // Query the OPENED HANDLE, not the potentially replaced pathname.
    // Windows FILE_INFO_BY_HANDLE_CLASS::FileAttributeTagInfo = 9.
    if unsafe {
        GetFileInformationByHandleEx(
            held.0, 9, (&mut info as *mut AttributeTag).cast(),
            std::mem::size_of::<AttributeTag>() as u32,
        )
    } == 0 || info.flags & FILE_ATTRIBUTE_DIRECTORY == 0
        || info.flags & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(B0Error::KeyUnavailable);
    }
    Ok(held)
}
fn ssh_system_binary() -> Result<PathBuf, B0Error> {
    // Do not execute an ssh.exe chosen by PATH, current working directory,
    // environment, AppData, a remote host or a reparse point.
    let mut buffer = [0u16; 1024];
    let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if len == 0 || len >= buffer.len() {
        return Err(B0Error::InspectionUnavailable);
    }
    let directory = PathBuf::from(OsString::from_wide(&buffer[..len]));
    let exe = directory.join("OpenSSH").join("ssh.exe");
    let md = fs::symlink_metadata(&exe).map_err(|_| B0Error::InspectionUnavailable)?;
    if !md.is_file() || md.file_type().is_symlink() {
        return Err(B0Error::InspectionUnavailable);
    }
    Ok(exe)
}
fn config_path(value: &Path) -> Result<String, B0Error> {
    let raw = value.to_str().ok_or(B0Error::KeyUnavailable)?;
    // Windows std::env::temp_dir() can use an extended-length \\?\ prefix.
    // Use the equivalent drive-absolute spelling for the system ssh.exe;
    // never permit network UNC or SSH-config token expansion.
    let s = raw.strip_prefix(r"\\?\").unwrap_or(raw);
    let bytes = s.as_bytes();
    let drive_absolute = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let bad_token = s.chars().any(|c| c.is_whitespace()
        || c.is_control()
        || matches!(c, '%' | '"' | '\'' | ';' | '#' | '='));
    if !value.is_absolute() || !drive_absolute || bad_token {
        if cfg!(debug_assertions)
            && std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true")
            && std::env::var("AKULA_B0C_CI_DIAG").ok().as_deref()
                == Some("LOOPBACK_ONLY")
        {
            // Shape only; do not print TEMP, user names or credentials.
            eprintln!(
                "B0C_PATH_CLASS abs={} drive={} bad_token={} len={} extended={}",
                value.is_absolute(), drive_absolute, bad_token,
                bytes.len(), raw.len() != s.len()
            );
        }
        return Err(pin_refused("config_path_policy"));
    }
    Ok(s.replace('\\', "/"))
}

struct PinnedKnownHosts {
    dir: PathBuf,
    path: PathBuf,
    handle: Option<File>,
    parent: Option<HeldDirectory>,
}
impl PinnedKnownHosts {
    fn create(id: Uuid, port: u16, encoded: &str) -> Result<Self, B0Error> {
        let temp = std::env::temp_dir();
        // A user-supplied TEMP with shell/config metacharacters is not a safe
        // place to create a known_hosts file for a privileged helper.
        let root = config_path(&temp)?;
        if root.len() > 768 {
            return Err(B0Error::KeyUnavailable);
        }
        let dir = temp.join(format!(
            "akula-b0c-{}-{}", id.simple(), Uuid::new_v4().simple()
        ));
        fs::create_dir(&dir).map_err(|e| pin_io("new_temp_dir", e))?;
        let pinned = (|| {
            let handle = pin_directory(&dir).map_err(|e| {
                report_local_ci("pin_parent_handle", None);
                e
            })?;
            let path = dir.join("known_hosts");
            // OpenSSH may index a non-standard SSH port under
            // [HostKeyAlias]:port rather than the bare alias. Pin BOTH
            // names to the SAME independent public key, never TOFU.
            let alias = format!("akula-b0c-{}", id.simple());
            let expected = format!(
                "{alias} ssh-ed25519 {encoded}\n[{alias}]:{port} ssh-ed25519 {encoded}\n"
            );
            let mut initial = OpenOptions::new()
                .write(true).create_new(true)
                .share_mode(FILE_SHARE_READ)
                .open(&path).map_err(|e| pin_io("new_known_hosts", e))?;
            initial.write_all(expected.as_bytes())
                .map_err(|e| pin_io("known_hosts_write", e))?;
            initial.sync_all().map_err(|e| pin_io("known_hosts_fsync", e))?;
            drop(initial);
            // Pin with READ-ONLY access. Holding a write-capable file handle
            // prevents some Windows OpenSSH clients from opening their own
            // read-only HANDLE under FILE_SHARE_READ.
            let mut file = OpenOptions::new()
                .read(true).share_mode(FILE_SHARE_READ)
                .open(&path).map_err(|e| pin_io("readonly_reopen", e))?;
            let mut check = Vec::new();
            (&mut file).take(4096).read_to_end(&mut check)
                .map_err(|e| pin_io("known_hosts_readback", e))?;
            if check.as_slice() != expected.as_bytes() {
                return Err(pin_refused("known_hosts_byte_mismatch"));
            }
            if fs::symlink_metadata(&path)
                .map_err(|e| pin_io("known_hosts_metadata", e))?
                .file_type().is_symlink()
            {
                return Err(pin_refused("known_hosts_symlink"));
            }
            // From now until child exit, the read-only pinned HANDLE denies
            // writes/deletes, including replacement under the known_hosts name.
            Ok(Self { dir: dir.clone(), path, handle: Some(file), parent: Some(handle) })
        })();
        if pinned.is_err() { let _ = fs::remove_dir(&dir); }
        pinned
    }
}
impl Drop for PinnedKnownHosts {
    fn drop(&mut self) {
        // Closing the handles restores delete access solely for cleanup.
        drop(self.handle.take());
        drop(self.parent.take());
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.dir);
    }
}

/// Actual signed SSH KEX and read-only facts, never installation authority.
/// Private identity is deliberately supplied only by an already running
/// Windows OpenSSH ssh-agent. No Akula setting contains a private key.
/// Read-only KEX/preflight and independent release review errors.
/// Neither variant authorizes remote installation or sudo.
#[cfg(feature = "server-install-review")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallReviewProbeError {
    Host(B0Error),
    Release(crate::server_release_lab::review::ReviewError),
}

pub struct WindowsOpenSshPinnedProbe {
    target: ServerTarget,
    id: Uuid,
    wire: Vec<u8>,
    known: PinnedKnownHosts,
    ssh_exe: PathBuf,
}
impl WindowsOpenSshPinnedProbe {
    /// An operator-provided, PUBLIC-only settings draft may request a
    /// read-only SSH preflight. No package, approval or installer plan is
    /// fabricated; a positive result describes the observed host facts only.
    /// Remote commands are fixed; the transport checks real signed SSH KEX.
    pub fn inspect_readonly_target(
        target: &ServerTarget,
        independently_pinned_host_public: &str,
    ) -> Result<ReadOnlyHostFacts, B0Error> {
        // No release PackagePin, fake installer manifest, sudo RPC, user
        // credential or owner approval is created for this inspection.
        let mut transport = Self::for_pinned_target(
            target.clone(), Uuid::new_v4(), independently_pinned_host_public,
        )?;
        transport.handshake_public_key(target)?;
        transport.inspect_host()
    }

    /// Verify actual OS SSH KEX and typed Debian/Ubuntu preflight, then
    /// independently verify B1a signed Node bytes for an immutable UI plan.
    ///
    /// A fingerprint-only settings profile can discover a candidate key via
    /// OS ssh-keyscan; it is accepted ONLY if SHA256 matches the independently
    /// pinned fingerprint and OpenSSH authenticates a signed KEX.
    ///
    /// Neither this call nor the returned PreparedReview is owner-signed
    /// B1b1b approval, remote installation authority or systemd activation.
    #[cfg(feature = "server-install-review")]
    pub fn prepare_signed_node_review<R: Read>(
        input: &B0Input,
        independently_pinned_host_public: Option<&str>,
        release: &crate::server_release_lab::SignedRelease,
        trusted_signer: &crate::server_release_lab::TrustedReleaseSigner,
        independently_persisted_floor: crate::server_release_lab::ReleaseFloor,
        node_package: &mut R,
        now_unix: u64,
        cancelled: impl Fn() -> bool,
    ) -> Result<crate::server_release_lab::review::PreparedReview, InstallReviewProbeError> {
        if cancelled() {
            return Err(InstallReviewProbeError::Host(B0Error::Cancelled));
        }
        let public = match independently_pinned_host_public {
            Some(key) => key.to_string(),
            None => scan::discover_matching_ed25519(&input.target)
                .map_err(InstallReviewProbeError::Host)?,
        };
        let mut probe = Self::new(input, &public)
            .map_err(InstallReviewProbeError::Host)?;
        let plan = crate::server_bootstrap_lab::preview_node_v2(
            input, &mut probe, &cancelled,
        ).map_err(InstallReviewProbeError::Host)?;
        if cancelled() {
            return Err(InstallReviewProbeError::Host(B0Error::Cancelled));
        }
        crate::server_release_lab::review::prepare_signed_review(
            plan, release, trusted_signer, independently_persisted_floor,
            node_package, now_unix,
        ).map_err(InstallReviewProbeError::Release)
    }

    pub fn new(input: &B0Input, trusted_host_public: &str) -> Result<Self, B0Error> {
        input.package.validate()?;
        Self::for_pinned_target(
            input.target.clone(), input.install_id, trusted_host_public,
        )
    }

    fn for_pinned_target(
        target: ServerTarget, id: Uuid, trusted_host_public: &str,
    ) -> Result<Self, B0Error> {
        target.validate()?;
        if id.is_nil() { return Err(B0Error::InvalidTarget); }
        let mut parts = trusted_host_public.split_ascii_whitespace();
        if parts.next() != Some("ssh-ed25519") { return Err(B0Error::KeyUnavailable); }
        let encoded = parts.next().ok_or(B0Error::KeyUnavailable)?;
        if parts.next().is_some() { return Err(B0Error::KeyUnavailable); }
        let wire = decode_ed25519_wire(encoded)?;
        let hash = digest::digest(&digest::SHA256, &wire);
        if constant_time::verify_slices_are_equal(
            hash.as_ref(), &target.trusted_key_sha256,
        ).is_err() { return Err(B0Error::HostKeyMismatch); }
        let ssh_exe = ssh_system_binary()?;
        let known = PinnedKnownHosts::create(id, target.ssh_port, encoded)?;
        Ok(Self {
            target, id,
            wire, known, ssh_exe,
        })
    }
    /// Share the exact independent host-key pin with the opt-in root stage.
    /// No password, TOFU, ProxyJump, user-supplied shell or key file.
    fn pinned_ssh_command(&self, use_stdin: bool) -> Result<Command, B0Error> {
        let known = config_path(&self.known.path)?;
        let mut ssh = Command::new(&self.ssh_exe);
        ssh.stdout(Stdio::piped()).stderr(Stdio::piped())
            .env_remove("SSH_ASKPASS").env_remove("GIT_SSH_COMMAND")
            .env_remove("SSH_AUTH_SOCK")
            .args(["-F", "none", "-T"]);
        if !use_stdin {
            ssh.arg("-n");
        }
        ssh.args(["-o", "BatchMode=yes"])
            .args(["-o", "StrictHostKeyChecking=yes"])
            .args(["-o", "HostKeyAlgorithms=ssh-ed25519"])
            .args(["-o", "GlobalKnownHostsFile=NUL"])
            .args(["-o", "KnownHostsCommand=none"])
            .args(["-o", "UpdateHostKeys=no"])
            .args(["-o", "VerifyHostKeyDNS=no"])
            .args(["-o", "CheckHostIP=no"])
            .args(["-o", "ProxyCommand=none"])
            .args(["-o", "ProxyJump=none"])
            .args(["-o", "CanonicalizeHostname=no"])
            .args(["-o", "GSSAPIAuthentication=no"])
            .args(["-o", "KbdInteractiveAuthentication=no"])
            .args(["-o", "PasswordAuthentication=no"])
            .args(["-o", "PubkeyAuthentication=yes"])
            .args(["-o", "PreferredAuthentications=publickey"])
            // No user-supplied SSH_AUTH_SOCK: let Windows OpenSSH use its
            // standard local ssh-agent service without reading a key file.
            .args(["-o", "IdentitiesOnly=no"])
            .args(["-o", "CertificateFile=none"])
            .args(["-o", "ControlMaster=no"])
            .args(["-o", "ClearAllForwardings=yes"])
            .args(["-o", "ForwardAgent=no"])
            .args(["-o", "ForwardX11=no"])
            .args(["-o", "PermitLocalCommand=no"])
            .args(["-o", "RequestTTY=no"])
            .args(["-o", "ConnectTimeout=5"])
            .args(["-o", "ConnectionAttempts=1"])
            .args(["-o", "NumberOfPasswordPrompts=0"])
            .args(["-o", "LogLevel=ERROR"])
            .arg("-o").arg(format!("UserKnownHostsFile={known}"))
            .arg("-o").arg(format!("HostKeyAlias=akula-b0c-{}", self.id.simple()))
            .arg("-p").arg(self.target.ssh_port.to_string())
            .arg("-l").arg(&self.target.ssh_user)
            .arg("--").arg(&self.target.host);
        Ok(ssh)
    }

    fn fixed_read(&self, command: &'static str) -> Result<String, B0Error> {
        let mut ssh = self.pinned_ssh_command(false)?;
        ssh.stdin(Stdio::null()).arg(command);
        let mut child = ssh.spawn().map_err(|_| B0Error::InspectionUnavailable)?;
        let stdout = child.stdout.take().ok_or(B0Error::InspectionUnavailable)?;
        let stderr = child.stderr.take().ok_or(B0Error::InspectionUnavailable)?;
        let reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(MAX_OUTPUT + 1).read_to_end(&mut bytes);
            bytes
        });
        let error_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.take(4097).read_to_end(&mut bytes);
            bytes
        });
        let deadline = Instant::now() + DEADLINE;
        let status = loop {
            if let Some(outcome) = child.try_wait()
                .map_err(|_| B0Error::InspectionUnavailable)?
            { break outcome; }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                let _ = error_reader.join();
                return Err(B0Error::InspectionUnavailable);
            }
            thread::sleep(Duration::from_millis(15));
        };
        let out = reader.join().map_err(|_| B0Error::InspectionUnavailable)?;
        let diagnostics = error_reader.join().map_err(|_| B0Error::InspectionUnavailable)?;
        if !status.success() || out.len() as u64 > MAX_OUTPUT {
            // NEVER surface remote stderr in an installed build. Only a
            // disposable GitHub Windows loopback test can print its bounded
            // escaped diagnostic, so the test can distinguish SSH config,
            // agent failure, KEX rejection and a broken Python fake server.
            if cfg!(debug_assertions)
                && self.target.host == "127.0.0.1"
                && std::env::var("GITHUB_ACTIONS").ok().as_deref() == Some("true")
                && std::env::var("AKULA_B0C_CI_DIAG").ok().as_deref()
                    == Some("LOOPBACK_ONLY")
            {
                let msg = String::from_utf8_lossy(&diagnostics);
                eprintln!("B0C_LOCAL_CI_SSH_EXIT={:?}", status.code());
                eprintln!("B0C_LOCAL_CI_SSH_STDERR_ESCAPED={:?}", msg.chars().take(1200).collect::<String>());
            }
            return Err(B0Error::InspectionUnavailable);
        }
        String::from_utf8(out).map_err(|_| B0Error::InspectionUnavailable)
    }
}
impl PreviewTransport for WindowsOpenSshPinnedProbe {
    fn handshake_public_key(&mut self, target: &ServerTarget) -> Result<Vec<u8>, B0Error> {
        if target != &self.target { return Err(B0Error::InvalidTarget); }
        // A successful ssh.exe authenticated session proves *this* host-key
        // signature passed the real SSH KEX against the one pinned known_hosts.
        self.fixed_read("true")?;
        Ok(self.wire.clone())
    }
    fn inspect_host(&mut self) -> Result<ReadOnlyHostFacts, B0Error> {
        parse_facts(&self.fixed_read(FACTS_COMMAND)?)
    }
}
fn decode_ed25519_wire(b64: &str) -> Result<Vec<u8>, B0Error> {
    if b64.len() != 68 { return Err(B0Error::KeyUnavailable); }
    fn digit(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62), b'/' => Some(63), _ => None,
        }
    }
    let mut wire = Vec::with_capacity(51);
    for g in b64.as_bytes().chunks_exact(4) {
        let (a,b,c,d) = (
            digit(g[0]).ok_or(B0Error::KeyUnavailable)?,
            digit(g[1]).ok_or(B0Error::KeyUnavailable)?,
            digit(g[2]).ok_or(B0Error::KeyUnavailable)?,
            digit(g[3]).ok_or(B0Error::KeyUnavailable)?,
        );
        wire.extend_from_slice(&[
            ((a<<2)|(b>>4)) as u8,
            ((b<<4)|(c>>2)) as u8,
            ((c<<6)|d) as u8,
        ]);
    }
    if wire.len() != 51 || &wire[..4] != b"\0\0\0\x0b"
        || &wire[4..15] != b"ssh-ed25519"
        || &wire[15..19] != b"\0\0\0\x20"
    { return Err(B0Error::KeyUnavailable); }
    Ok(wire)
}
fn parse_facts(raw: &str) -> Result<ReadOnlyHostFacts, B0Error> {
    if raw.len() > MAX_OUTPUT as usize { return Err(B0Error::InspectionUnavailable); }
    let mut fields = BTreeMap::new();
    for line in raw.lines() {
        let (k,v) = line.split_once('=').ok_or(B0Error::InspectionUnavailable)?;
        if !matches!(k, "AKULA_B0_OS" | "AKULA_B0_ARCH" |
            "AKULA_B0_SYSTEMD" | "AKULA_B0_SUDO" | "AKULA_B0_CPU" |
            "AKULA_B0_RAM" | "AKULA_B0_DISK"
        ) || v.len() > 100 || fields.insert(k,v).is_some() {
            return Err(B0Error::InspectionUnavailable);
        }
    }
    if fields.len() != 7 { return Err(B0Error::InspectionUnavailable); }
    let get = |k: &str| fields.get(k).copied().ok_or(B0Error::InspectionUnavailable);
    let os = match get("AKULA_B0_OS")?
        .strip_prefix("ID=").ok_or(B0Error::InspectionUnavailable)?
        .trim_matches('"')
    {
        "debian" => SupportedOs::Debian,
        "ubuntu" => SupportedOs::Ubuntu,
        _ => SupportedOs::Other,
    };
    let arch = match get("AKULA_B0_ARCH")? {
        "x86_64" => MachineArch::X86_64,
        "aarch64" => MachineArch::Aarch64,
        _ => MachineArch::Other,
    };
    let yes_no = |s| match s {
        "yes" => Ok(true), "no" => Ok(false),
        _ => Err(B0Error::InspectionUnavailable),
    };
    let num = |k: &str| get(k)?.parse::<u64>()
        .map_err(|_| B0Error::InspectionUnavailable);
    Ok(ReadOnlyHostFacts {
        os, arch, has_systemd: yes_no(get("AKULA_B0_SYSTEMD")?)?,
        has_sudo_rights: yes_no(get("AKULA_B0_SUDO")?)?,
        cpu_count: u16::try_from(num("AKULA_B0_CPU")?)
            .map_err(|_| B0Error::InspectionUnavailable)?,
        ram_mib: num("AKULA_B0_RAM")?,
        free_disk_mib: num("AKULA_B0_DISK")?,
    })
}
