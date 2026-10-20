//! REAL Windows named-pipe transport with all broker permissions OFF.
//! This is not a service installation or a P2P File Gate authorization.
//!
//! The process must already run as LocalSystem, the pipe must be the first
//! server instance, remote clients are refused by the OS, and its DACL grants
//! only FILE_READ_DATA/FILE_WRITE_DATA/SYNCHRONIZE to interactive clients,
//! NOT FILE_CREATE_PIPE_INSTANCE. Each received message is associated with an
//! impersonated Windows thread token; a failed impersonation hard-denies.
//!
//! Even after all those checks, this implementation ONLY returns HOLD.
//! A privileged installer-owned registry, authenticated service identity at
//! the client, durable policy epochs, device enrollment and grant issuance
//! are absent. Do not connect the installed Akula application to this probe.

#![cfg(windows)]

use crate::{deny_frame, REQUEST_BYTES};
use std::ffi::c_void;
use std::io::{self, ErrorKind};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

type Handle = *mut c_void;
const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_USER_CLASS: u32 = 1;
const WIN_LOCAL_SYSTEM_SID: u32 = 22;
const ERROR_PIPE_CONNECTED: u32 = 535;
const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_TYPE_MESSAGE: u32 = 0x0000_0004;
const PIPE_READMODE_MESSAGE: u32 = 0x0000_0002;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit_handle: i32,
}

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}
#[repr(C)]
struct TokenUser {
    user: SidAndAttributes,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentThread() -> Handle;
    fn GetLastError() -> u32;
    fn CloseHandle(handle: Handle) -> i32;
    fn LocalFree(memory: Handle) -> Handle;
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        output_buffer: u32,
        input_buffer: u32,
        default_timeout: u32,
        attributes: *const SecurityAttributes,
    ) -> Handle;
    fn ConnectNamedPipe(handle: Handle, overlapped: Handle) -> i32;
    fn ReadFile(handle: Handle, output: *mut c_void, size: u32, read: *mut u32, overlapped: Handle) -> i32;
    fn WriteFile(handle: Handle, input: *const c_void, size: u32, written: *mut u32, overlapped: Handle) -> i32;
    fn FlushFileBuffers(handle: Handle) -> i32;
    fn DisconnectNamedPipe(handle: Handle) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, result: *mut Handle) -> i32;
    fn OpenThreadToken(thread: Handle, access: u32, open_as_self: i32, result: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, info_class: u32, output: Handle, size: u32, required: *mut u32) -> i32;
    fn IsValidSid(sid: Handle) -> i32;
    fn EqualSid(left: Handle, right: Handle) -> i32;
    fn CreateWellKnownSid(kind: u32, domain: Handle, output: Handle, size: *mut u32) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16,
        revision: u32,
        descriptor: *mut Handle,
        descriptor_size: *mut u32,
    ) -> i32;
    fn ImpersonateNamedPipeClient(pipe: Handle) -> i32;
    fn RevertToSelf() -> i32;
}

struct OwnedHandle(Handle);
impl OwnedHandle {
    fn new(value: Handle) -> io::Result<Self> {
        if value.is_null() || value == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(value))
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: Created by Win32 API; this wrapper owns exactly one handle.
        unsafe { CloseHandle(self.0) };
    }
}

struct OwnedSecurityDescriptor(Handle);
impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: The descriptor is allocated by ConvertStringSecurityDescriptor...
        unsafe { LocalFree(self.0) };
    }
}

struct ImpersonationGuard;
impl Drop for ImpersonationGuard {
    fn drop(&mut self) {
        // If returning to the service identity fails, execution must stop:
        // continuing with an unknown security context is not acceptable.
        if unsafe { RevertToSelf() } == 0 {
            std::process::abort();
        }
    }
}

fn denied(message: &str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, message)
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

fn token_user_sid_is_system(token: Handle) -> io::Result<bool> {
    let mut bytes = 0u32;
    // SAFETY: Initial insufficient-buffer call requests the required length.
    unsafe {
        GetTokenInformation(token, TOKEN_USER_CLASS, ptr::null_mut(), 0, &mut bytes);
    }
    if bytes < size_of::<TokenUser>() as u32 || bytes > 4096 {
        return Err(denied("unexpected Windows token size"));
    }
    let words = (bytes as usize).div_ceil(size_of::<usize>());
    let mut storage = vec![0usize; words];
    if unsafe {
        GetTokenInformation(
            token, TOKEN_USER_CLASS, storage.as_mut_ptr().cast(),
            (words * size_of::<usize>()) as u32, &mut bytes
        )
    } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: A successful TokenUser query initialized the aligned header.
    let user_sid = unsafe { (*(storage.as_ptr().cast::<TokenUser>())).user.sid };
    if user_sid.is_null() || unsafe { IsValidSid(user_sid) } == 0 {
        return Err(denied("invalid token SID"));
    }
    let mut system = [0usize; 16];
    let mut sid_bytes = (system.len() * size_of::<usize>()) as u32;
    if unsafe {
        CreateWellKnownSid(
            WIN_LOCAL_SYSTEM_SID, ptr::null_mut(), system.as_mut_ptr().cast(), &mut sid_bytes
        )
    } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { EqualSid(user_sid, system.as_mut_ptr().cast()) } != 0)
}

fn require_system_service_identity() -> io::Result<()> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle::new(token)?;
    if !token_user_sid_is_system(token.0)? {
        return Err(denied("probe requires a pre-existing LocalSystem service identity"));
    }
    Ok(())
}

/// Called ONLY after a message was read from the connected pipe.
/// Impersonation is mandatory, but no current request ever authorizes access.
fn observe_client_token_after_message(pipe: Handle) -> io::Result<()> {
    if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
        return Err(denied("Windows named-pipe client impersonation failed"));
    }
    let guard = ImpersonationGuard;
    let mut token = ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle::new(token)?;
    // This is an OS token-presence check, NOT authorization for DeviceId or
    // replica. Independently verified client enrollment remains a blocker.
    let _observed_system_sid = token_user_sid_is_system(token.0)?;
    drop(token);
    drop(guard);
    Ok(())
}

/// Single-request, non-shipping real Windows pipe server. It cannot provision,
/// load or change any authority state; no variant ever allows file mutation.
pub fn run_one_denied_connection() -> io::Result<()> {
    // Check the real process token BEFORE creating the pipe name. Calling
    // this binary as an ordinary user must not create a fake "broker" server.
    require_system_service_identity()?;

    // Named-pipe FILE_CREATE_PIPE_INSTANCE shares the FILE_APPEND_DATA bit.
    // Do not grant GENERIC_WRITE to Interactive Users: that could allow them
    // to create another server instance. Grant only READ_DATA, WRITE_DATA and
    // SYNCHRONIZE (0x00100003); SYSTEM and Admins manage the server object.
    let sddl = wide("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00100003;;;IU)");
    let mut raw_descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(), 1, &mut raw_descriptor, ptr::null_mut()
        )
    } == 0 {
        return Err(io::Error::last_os_error());
    }
    let descriptor = OwnedSecurityDescriptor(raw_descriptor);
    let attributes = SecurityAttributes {
        length: size_of::<SecurityAttributes>() as u32,
        descriptor: descriptor.0,
        inherit_handle: 0,
    };
    let name = wide(r"\\.\pipe\AkulaHostBrokerResearch.v1");
    let pipe = OwnedHandle::new(unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS,
            1, 512, 512, 5000, &attributes,
        )
    })?;

    let connected = unsafe { ConnectNamedPipe(pipe.0, ptr::null_mut()) };
    if connected == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
        return Err(io::Error::last_os_error());
    }

    let result = (|| -> io::Result<()> {
        let mut input = [0u8; REQUEST_BYTES];
        let mut count = 0u32;
        // A message longer than 32 bytes yields ERROR_MORE_DATA and is denied.
        if unsafe {
            ReadFile(
                pipe.0, input.as_mut_ptr().cast(), REQUEST_BYTES as u32,
                &mut count, ptr::null_mut()
            )
        } == 0 {
            return Err(denied("broker probe refused oversized or unreadable message"));
        }
        observe_client_token_after_message(pipe.0)?;
        let response = deny_frame(&input[..count as usize]);
        let mut written = 0u32;
        if unsafe {
            WriteFile(
                pipe.0, response.as_ptr().cast(),
                response.len() as u32, &mut written, ptr::null_mut()
            )
        } == 0 || written as usize != response.len() {
            return Err(io::Error::last_os_error());
        }
        unsafe { FlushFileBuffers(pipe.0) };
        Ok(())
    })();
    // This is a single-connection probe, not an always-on daemon/service.
    unsafe { DisconnectNamedPipe(pipe.0) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unprivileged_process_must_not_create_broker_pipe() {
        // Hosted Windows CI runs as a regular/elevated user, not the actual
        // LocalSystem service. If a future runner IS LocalSystem, skip rather
        // than block on ConnectNamedPipe without an independently issued
        // positive service fixture.
        if require_system_service_identity().is_err() {
            assert!(run_one_denied_connection().is_err());
        }
    }
}
