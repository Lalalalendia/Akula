//! Experimental Windows SCM lifecycle with ZERO authority.
//! A joined custody+pipe mode pins a protected installer-like marker BEFORE
//! exposing a deny-only OS-owned pipe in the SAME LocalSystem service.
//! No grant, enrollment, File Gate writer or production installation.

#![cfg(windows)]

use super::windows_cancellable_pipe::CancelablePipe;
use super::windows_server::require_system_service_identity;
use super::windows_store_custody::CustodyWitness;
use std::ffi::c_void;
use std::io::{self, ErrorKind};
#[cfg(test)]
use std::mem::size_of;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

type Handle = *mut c_void;
type StatusHandle = *mut c_void;
const SERVICE_WIN32_OWN_PROCESS: u32 = 0x10;
const SERVICE_STOPPED: u32 = 1;
const SERVICE_START_PENDING: u32 = 2;
const SERVICE_STOP_PENDING: u32 = 3;
const SERVICE_RUNNING: u32 = 4;
const SERVICE_ACCEPT_STOP: u32 = 1;
const SERVICE_ACCEPT_SHUTDOWN: u32 = 4;
const SERVICE_CONTROL_STOP: u32 = 1;
const SERVICE_CONTROL_SHUTDOWN: u32 = 5;
const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;
const ERROR_SERVICE_SPECIFIC_ERROR: u32 = 1066;
const WAIT_OBJECT_0: u32 = 0;
const INFINITE: u32 = u32::MAX;
const SCM_NAME: &str = "AkulaHostBrokerResearch";
static STOP_SIGNAL: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static IPC_TEST_MODE: AtomicBool = AtomicBool::new(false);
static CUSTODY_TEST_MODE: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct ServiceTableEntryW {
    name: *mut u16,
    main: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ServiceStatus {
    service_type: u32,
    current_state: u32,
    accepted_controls: u32,
    win32_exit_code: u32,
    service_specific_exit_code: u32,
    checkpoint: u32,
    wait_hint: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn StartServiceCtrlDispatcherW(table: *const ServiceTableEntryW) -> i32;
    fn RegisterServiceCtrlHandlerExW(
        name: *const u16,
        handler: Option<unsafe extern "system" fn(u32, u32, Handle, Handle) -> u32>,
        context: Handle,
    ) -> StatusHandle;
    fn SetServiceStatus(handle: StatusHandle, status: *const ServiceStatus) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(attrs: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

struct OwnedEvent(Handle);
impl OwnedEvent {
    fn create() -> io::Result<Self> {
        let raw = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(raw))
    }
}
impl Drop for OwnedEvent {
    fn drop(&mut self) {
        STOP_SIGNAL.store(ptr::null_mut(), Ordering::SeqCst);
        unsafe { CloseHandle(self.0) };
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
fn report(
    handle: StatusHandle,
    state: u32,
    controls: u32,
    error: u32,
    checkpoint: u32,
    wait_hint: u32,
) -> io::Result<()> {
    let status = ServiceStatus {
        service_type: SERVICE_WIN32_OWN_PROCESS,
        current_state: state,
        accepted_controls: controls,
        win32_exit_code: if error == 0 {
            0
        } else {
            ERROR_SERVICE_SPECIFIC_ERROR
        },
        service_specific_exit_code: error,
        checkpoint,
        wait_hint,
    };
    if unsafe { SetServiceStatus(handle, &status) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The SCM invokes this handler on a dispatcher thread. It performs no IO
/// except SetEvent. It NEVER authorizes a peer or touches a host registry.
unsafe extern "system" fn control_handler(
    control: u32,
    _event_type: u32,
    _event_data: Handle,
    _context: Handle,
) -> u32 {
    if control != SERVICE_CONTROL_STOP && control != SERVICE_CONTROL_SHUTDOWN {
        return ERROR_CALL_NOT_IMPLEMENTED;
    }
    let event = STOP_SIGNAL.load(Ordering::SeqCst);
    if event.is_null() || unsafe { SetEvent(event) } == 0 {
        return ERROR_CALL_NOT_IMPLEMENTED;
    }
    0
}

fn run_service() -> io::Result<()> {
    let event = OwnedEvent::create()?;
    if STOP_SIGNAL
        .compare_exchange(ptr::null_mut(), event.0, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "already dispatched",
        ));
    }
    let name = wide(SCM_NAME);
    let handler = unsafe {
        RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(control_handler), ptr::null_mut())
    };
    if handler.is_null() {
        return Err(io::Error::last_os_error());
    }
    report(handler, SERVICE_START_PENDING, 0, 0, 1, 5000)?;
    // Actual pre-existing installer custody is a prerequisite to reporting
    // RUNNING. No appdata fallback, provisioning, grants or state writes.
    let _custody = if CUSTODY_TEST_MODE.load(Ordering::SeqCst) {
        if let Err(error) = require_system_service_identity() {
            let _ = report(handler, SERVICE_STOPPED, 0, 1, 0, 0);
            return Err(error);
        }
        match CustodyWitness::inspect_fixed() {
            Ok(verified) => Some(verified),
            Err(error) => {
                let _ = report(handler, SERVICE_STOPPED, 0, 1, 0, 0);
                return Err(error);
            }
        }
    } else {
        None
    };
    // A required custody witness is already held and cannot be bypassed by
    // falling back to an unchecked IPC service if inspection fails.
    // The named pipe must be created and secured BEFORE SCM reports RUNNING.
    // A service that cannot bind its exact first-instance pipe fails closed.
    let pipe = if IPC_TEST_MODE.load(Ordering::SeqCst) {
        match CancelablePipe::new() {
            Ok(p) => Some(p),
            Err(error) => {
                let _ = report(handler, SERVICE_STOPPED, 0, 1, 0, 0);
                return Err(error);
            }
        }
    } else {
        None
    };
    report(
        handler,
        SERVICE_RUNNING,
        SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN,
        0,
        0,
        0,
    )?;
    let outcome = if let Some(pipe) = pipe {
        // Stop and blocked/idle client are both serviced by overlapped I/O.
        pipe.run_until_stop(event.0, || {
            if let Some(ref witness) = _custody {
                witness.recheck_fixed_names()?;
            }
            Ok(())
        })
    } else {
        let state = unsafe { WaitForSingleObject(event.0, INFINITE) };
        if state == WAIT_OBJECT_0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    };
    if let Err(error) = outcome {
        let _ = report(handler, SERVICE_STOPPED, 0, 1, 0, 0);
        return Err(error);
    }
    report(handler, SERVICE_STOP_PENDING, 0, 0, 1, 5000)?;
    report(handler, SERVICE_STOPPED, 0, 0, 0, 0)?;
    Ok(())
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
    // Never unwind across the Win32 service FFI boundary.
    let result = std::panic::catch_unwind(run_service);
    if !matches!(result, Ok(Ok(()))) {
        eprintln!("Akula research SCM service stopped after initialization failure");
    }
}

/// Entry point exclusively for processes actually launched by SCM.
/// Interactive invocation returns ERROR_FAILED_SERVICE_CONTROLLER_CONNECT.
pub fn dispatch_scm_service() -> io::Result<()> {
    dispatch_scm(false)
}

/// Testing a cancellable pipe under actual SCM. This remains a separate
/// temporary Windows service mode: no request can grant P2P authority.
pub fn dispatch_scm_pipe_service() -> io::Result<()> {
    dispatch_scm(true)
}

/// Research-only LocalSystem SCM mode checking installed marker custody.
pub fn dispatch_scm_custody_service() -> io::Result<()> {
    CUSTODY_TEST_MODE.store(true, Ordering::SeqCst);
    dispatch_scm(false)
}

/// Verify pinned, pre-existing OS custody BEFORE binding the secured named
/// pipe and reporting RUNNING. Both pinned handles remain held for the whole
/// pipe/SCM lifetime. This research mode cannot return grants or enroll roots.
pub fn dispatch_scm_custody_pipe_service() -> io::Result<()> {
    CUSTODY_TEST_MODE.store(true, Ordering::SeqCst);
    dispatch_scm(true)
}

fn dispatch_scm(pipe_mode: bool) -> io::Result<()> {
    IPC_TEST_MODE.store(pipe_mode, Ordering::SeqCst);
    let mut name = wide(SCM_NAME);
    let entries = [
        ServiceTableEntryW {
            name: name.as_mut_ptr(),
            main: Some(service_main),
        },
        ServiceTableEntryW {
            name: ptr::null_mut(),
            main: None,
        },
    ];
    if unsafe { StartServiceCtrlDispatcherW(entries.as_ptr()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_exact_win32_abi_size() {
        assert_eq!(size_of::<ServiceStatus>(), 28);
        assert_eq!(size_of::<ServiceTableEntryW>(), 2 * size_of::<usize>());
        assert!(STOP_SIGNAL.load(Ordering::SeqCst).is_null());
    }

    #[test]
    fn interactive_test_process_cannot_fake_scm_dispatcher() {
        // This process is not started by the Service Control Manager.
        assert!(dispatch_scm_service().is_err());
        assert!(STOP_SIGNAL.load(Ordering::SeqCst).is_null());
    }
}
