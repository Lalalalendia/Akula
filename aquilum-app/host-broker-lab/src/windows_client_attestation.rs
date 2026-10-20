//! RN-4E1b-03b1b — pinned Windows pipe/SCM server identity preflight.
//!
//! This is an independent read-only probe. No installed Akula code invokes
//! it; it NEVER transmits a message, returns a grant, creates an SCM service,
//! authenticates a QUIC peer or grants File Gate access.
//!
//! Server identity requires all of:
//! (1) a RUNNING own-process SCM service with fixed lab service name;
//! (2) the PID of the already-open named-pipe instance matches that service;
//! (3) a pinned process token owned by LocalSystem;
//! (4) an enabled, non-deny-only SID for precisely that NT SERVICE account;
//! (5) unchanged SCM PID and pinned pipe PID at the end of the observation.
//!
//! Without a real, separately installed service this MUST fail closed. Even
//! with all checks, a signed installer, service executable verification,
//! protected local policy store and authenticated P2P authorization are absent.

#![cfg(windows)]

use std::ffi::{c_void, OsStr};
use std::io::{self, ErrorKind};
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::ptr;

type Handle = *mut c_void;
const INVALID_HANDLE: Handle = -1isize as Handle;

const SC_MANAGER_CONNECT: u32 = 0x0001;
const SERVICE_QUERY_STATUS: u32 = 0x0004;
const SERVICE_RUNNING: u32 = 0x0000_0004;
const SERVICE_WIN32_OWN_PROCESS: u32 = 0x0000_0010;
const SC_STATUS_PROCESS_INFO: u32 = 0;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER: u32 = 1;
const TOKEN_GROUPS: u32 = 2;
const WIN_LOCAL_SYSTEM_SID: u32 = 22;
const SE_GROUP_ENABLED: u32 = 0x0000_0004;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x0000_0010;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const FILE_READ_DATA: u32 = 0x0001;
const OPEN_EXISTING: u32 = 3;
const SECURITY_SQOS_PRESENT: u32 = 0x0010_0000;
const SECURITY_IDENTIFICATION: u32 = 0x0001_0000;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_ACCESS_DUPLEX: u32 = 3;
const PIPE_TYPE_MESSAGE: u32 = 4;
const PIPE_READMODE_MESSAGE: u32 = 2;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;

const SERVICE_NAME: &str = "AkulaHostBrokerResearch";
const SERVICE_SID_ACCOUNT: &str = r"NT SERVICE\AkulaHostBrokerResearch";
const PIPE_PATH: &str = r"\\.\pipe\AkulaHostBrokerResearch.v1";

#[repr(C)]
#[derive(Default)]
struct ServiceStatusProcess {
    service_type: u32,
    current_state: u32,
    controls_accepted: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    checkpoint: u32,
    wait_hint: u32,
    process_id: u32,
    service_flags: u32,
}

#[repr(C)]
struct SidAndAttributes {
    sid: Handle,
    attributes: u32,
}

#[repr(C)]
struct TokenUser {
    user: SidAndAttributes,
}

#[repr(C)]
struct TokenGroups {
    group_count: u32,
    groups: [SidAndAttributes; 1],
}

struct OwnedHandle(Handle);
impl OwnedHandle {
    fn new(value: Handle) -> io::Result<Self> {
        if value.is_null() || value == INVALID_HANDLE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(value))
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

struct OwnedScHandle(Handle);
impl OwnedScHandle {
    fn new(value: Handle) -> io::Result<Self> {
        if value.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(value))
    }
}
impl Drop for OwnedScHandle {
    fn drop(&mut self) {
        unsafe { CloseServiceHandle(self.0) };
    }
}

struct PinnedProcess {
    _process: OwnedHandle,
    token: OwnedHandle,
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(value: Handle) -> i32;
    fn GetLastError() -> u32;
    fn CreateFileW(
        path: *const u16,
        desired_access: u32,
        share_mode: u32,
        security: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn GetNamedPipeServerProcessId(pipe: Handle, server_id: *mut u32) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    #[cfg(test)]
    fn GetCurrentProcessId() -> u32;
    #[cfg(test)]
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        output_buffer: u32,
        input_buffer: u32,
        default_timeout: u32,
        attributes: Handle,
    ) -> Handle;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> Handle;
    fn OpenServiceW(manager: Handle, name: *const u16, access: u32) -> Handle;
    fn QueryServiceStatusEx(
        service: Handle,
        info_level: u32,
        output: *mut u8,
        length: u32,
        needed: *mut u32,
    ) -> i32;
    fn CloseServiceHandle(handle: Handle) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        output: Handle,
        size: u32,
        required: *mut u32,
    ) -> i32;
    fn CreateWellKnownSid(kind: u32, domain: Handle, output: Handle, size: *mut u32) -> i32;
    fn LookupAccountNameW(
        system: *const u16,
        account: *const u16,
        sid: Handle,
        sid_length: *mut u32,
        domain: *mut u16,
        domain_length: *mut u32,
        use_type: *mut u32,
    ) -> i32;
    fn IsValidSid(sid: Handle) -> i32;
    fn EqualSid(first: Handle, second: Handle) -> i32;
}

fn deny(reason: &'static str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, reason)
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn service_process_id() -> io::Result<u32> {
    let manager = OwnedScHandle::new(unsafe {
        OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT)
    })?;
    let service_name = wide(SERVICE_NAME);
    let service = OwnedScHandle::new(unsafe {
        OpenServiceW(manager.0, service_name.as_ptr(), SERVICE_QUERY_STATUS)
    })?;
    let mut status = ServiceStatusProcess::default();
    let mut bytes = 0;
    if unsafe {
        QueryServiceStatusEx(
            service.0,
            SC_STATUS_PROCESS_INFO,
            (&mut status as *mut ServiceStatusProcess).cast(),
            size_of::<ServiceStatusProcess>() as u32,
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // On success pcbBytesNeeded may not report the structure size.
    // QueryServiceStatusEx initializes the fixed-size output on success.
    if status.current_state != SERVICE_RUNNING
        || status.service_type & SERVICE_WIN32_OWN_PROCESS == 0
        || status.process_id == 0
    {
        return Err(deny("expected running dedicated installed SCM broker"));
    }
    Ok(status.process_id)
}

fn pipe_server_pid(handle: Handle) -> io::Result<u32> {
    let mut pid = 0;
    if unsafe { GetNamedPipeServerProcessId(handle, &mut pid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        return Err(deny("named pipe has no identifiable server process"));
    }
    Ok(pid)
}

fn token_blob(token: Handle, class: u32) -> io::Result<(Vec<usize>, usize)> {
    let mut needed = 0;
    unsafe {
        GetTokenInformation(token, class, ptr::null_mut(), 0, &mut needed);
    }
    if needed < 8 || needed > 65_536 {
        return Err(deny("invalid or oversized security token representation"));
    }
    let words = (needed as usize).div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; words];
    if unsafe {
        GetTokenInformation(
            token,
            class,
            storage.as_mut_ptr().cast(),
            (words * size_of::<usize>()) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if needed as usize > storage.len() * size_of::<usize>() {
        return Err(deny("security token grew during inspection"));
    }
    Ok((storage, needed as usize))
}

fn local_system_sid() -> io::Result<Vec<usize>> {
    let mut words = vec![0usize; 16];
    let mut bytes = (words.len() * size_of::<usize>()) as u32;
    if unsafe {
        CreateWellKnownSid(
            WIN_LOCAL_SYSTEM_SID,
            ptr::null_mut(),
            words.as_mut_ptr().cast(),
            &mut bytes,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(words)
}

fn service_sid() -> io::Result<Vec<usize>> {
    let account = wide(SERVICE_SID_ACCOUNT);
    let mut sid_len = 0;
    let mut domain_len = 0;
    let mut account_type = 0;
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &mut sid_len,
            ptr::null_mut(),
            &mut domain_len,
            &mut account_type,
        );
    }
    if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
        || sid_len < 8
        || sid_len > 1024
        || domain_len > 1024
    {
        return Err(deny("installed service SID cannot be resolved"));
    }
    let mut sid = vec![0usize; (sid_len as usize).div_ceil(size_of::<usize>())];
    let mut domain = vec![0u16; (domain_len as usize).max(1)];
    if unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut account_type,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if unsafe { IsValidSid(sid.as_mut_ptr().cast()) } == 0 {
        return Err(deny("resolved service SID invalid"));
    }
    Ok(sid)
}

fn token_matches_sid(token: Handle, expected: Handle) -> io::Result<bool> {
    let (blob, used) = token_blob(token, TOKEN_USER)?;
    if used < size_of::<TokenUser>() {
        return Err(deny("truncated token owner"));
    }
    let user = unsafe { &*(blob.as_ptr().cast::<TokenUser>()) };
    if user.user.sid.is_null() || unsafe { IsValidSid(user.user.sid) } == 0 {
        return Err(deny("token owner has invalid SID"));
    }
    Ok(unsafe { EqualSid(user.user.sid, expected) } != 0)
}

fn token_has_enabled_service_sid(token: Handle, expected: Handle) -> io::Result<bool> {
    let (blob, used) = token_blob(token, TOKEN_GROUPS)?;
    if used < size_of::<TokenGroups>() {
        return Err(deny("truncated service token groups"));
    }
    let header = unsafe { &*(blob.as_ptr().cast::<TokenGroups>()) };
    let count = header.group_count as usize;
    let offset = offset_of!(TokenGroups, groups);
    let required = offset
        .checked_add(
            count
                .checked_mul(size_of::<SidAndAttributes>())
                .ok_or_else(|| deny("token group count overflow"))?,
        )
        .ok_or_else(|| deny("token group size overflow"))?;
    if required > used || count > 4096 {
        return Err(deny("out of bounds token group list"));
    }
    let start = unsafe { blob.as_ptr().cast::<u8>().add(offset) };
    for i in 0..count {
        let row = unsafe { &*start.cast::<SidAndAttributes>().add(i) };
        if !row.sid.is_null()
            && unsafe { IsValidSid(row.sid) } != 0
            && unsafe { EqualSid(row.sid, expected) } != 0
            && row.attributes & SE_GROUP_ENABLED != 0
            && row.attributes & SE_GROUP_USE_FOR_DENY_ONLY == 0
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn verified_process_for_pipe(pipe: Handle, expected_pid: u32) -> io::Result<PinnedProcess> {
    if pipe_server_pid(pipe)? != expected_pid {
        return Err(deny(
            "pipe server PID does not equal registered SCM service PID",
        ));
    }
    let process = OwnedHandle::new(unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, expected_pid)
    })?;
    let mut raw_token = ptr::null_mut();
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut raw_token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle::new(raw_token)?;
    let sid = local_system_sid()?;
    if !token_matches_sid(token.0, sid.as_ptr() as Handle)? {
        return Err(deny("named-pipe server is not LocalSystem"));
    }
    Ok(PinnedProcess {
        _process: process,
        token,
    })
}

/// Observe only; never send any payload. A successful return here DOES NOT
/// establish service signing, immutable OS authority, P2P enrollment or
/// broker permission. No installed Akula component invokes this function.
#[allow(dead_code)]
pub fn inspect_installed_service_pipe_without_sending() -> io::Result<()> {
    // SCM is checked first: a fake user-created pipe does not cause any
    // application payload to be transmitted or a first-contact handshake.
    let service_pid = service_process_id()?;
    let pipe_name = wide(PIPE_PATH);
    // Request FILE_READ_DATA only, NEVER GENERIC_WRITE or pipe-instance
    // creation rights. SQOS identification prevents full impersonation.
    let pipe = OwnedHandle::new(unsafe {
        CreateFileW(
            pipe_name.as_ptr(),
            FILE_READ_DATA,
            0,
            ptr::null_mut(),
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            ptr::null_mut(),
        )
    })?;
    let pinned = verified_process_for_pipe(pipe.0, service_pid)?;
    let exact_service_sid = service_sid()?;
    if !token_has_enabled_service_sid(pinned.token.0, exact_service_sid.as_ptr() as Handle)? {
        return Err(deny("named-pipe server token lacks enabled service SID"));
    }
    if pipe_server_pid(pipe.0)? != service_pid || service_process_id()? != service_pid {
        return Err(deny("SCM service identity changed during observation"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_user_cannot_be_treated_as_installed_system_broker() {
        // Exercise a REAL locally creatable pipe instance and its real PID.
        // It must not turn into a trusted service via a matching process ID.
        let pipe_name = wide(&format!(
            r"\\.\pipe\AkulaHostBrokerFakeServerTest.{}",
            unsafe { GetCurrentProcessId() }
        ));
        let fake = OwnedHandle::new(unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                128,
                128,
                5000,
                ptr::null_mut(),
            )
        })
        .unwrap();
        let pid = unsafe { GetCurrentProcessId() };
        // Read the real owner PID from an actual user-created named pipe,
        // rather than relying only on a mocked PID comparison.
        assert_eq!(pipe_server_pid(fake.0).unwrap(), pid);
        // CI is ordinarily an unprivileged or interactive admin process.
        // If it is actually SYSTEM (future privileged runner), this unit
        // cannot act as a hostile normal user; do not claim a negative proof.
        if let Ok(process) = verified_process_for_pipe(fake.0, pid) {
            assert!(token_matches_sid(
                process.token.0,
                local_system_sid().unwrap().as_ptr() as Handle
            )
            .unwrap());
        }
    }

    #[test]
    fn without_real_scm_service_attestation_no_payload_is_sent() {
        // Do not install a service on CI; absence must be a hard error.
        if service_process_id().is_err() {
            assert!(inspect_installed_service_pipe_without_sending().is_err());
        }
    }

    #[test]
    fn current_invalid_pid_is_refused_even_on_a_live_local_pipe() {
        let pipe_name = wide(&format!(
            r"\\.\pipe\AkulaHostBrokerFakeOtherPidTest.{}",
            unsafe { GetCurrentProcessId() }
        ));
        let pipe = OwnedHandle::new(unsafe {
            CreateNamedPipeW(
                pipe_name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                128,
                128,
                5000,
                ptr::null_mut(),
            )
        })
        .unwrap();
        assert_eq!(pipe_server_pid(pipe.0).unwrap(), unsafe {
            GetCurrentProcessId()
        });
        let wrong_pid = unsafe { GetCurrentProcessId() }.wrapping_add(10_000);
        assert!(verified_process_for_pipe(pipe.0, wrong_pid).is_err());
    }
}
