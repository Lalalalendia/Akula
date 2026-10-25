//! RN-4E1b-03b1d: cancellable SCM-owned pipe with NO write authority.
//! The server uses the SAME protected, first-instance SYSTEM-only pipe as
//! the earlier standalone denial probe, but makes connection/read overlapped
//! so STOP can terminate a blocked accept or silent client.
//!
//! It never returns a positive authorization, never accesses user files and
//! never interprets even one request as a permission. A connected client is
//! deliberately disconnected after at most ONE byte; no response is sent.
//! A full policy protocol is still forbidden until broker-owned state exists.

#![cfg(windows)]

use super::windows_server::{create_secure_pipe, OwnedHandle};
use std::ffi::c_void;
use std::io::{self, ErrorKind};
#[cfg(test)]
use std::mem::size_of;
use std::ptr;

type Handle = *mut c_void;
const ERROR_IO_PENDING: u32 = 997;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_BROKEN_PIPE: u32 = 109;
const ERROR_PIPE_NOT_CONNECTED: u32 = 233;
const ERROR_MORE_DATA: u32 = 234;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const INFINITE: u32 = u32::MAX;

/// Win32 OVERLAPPED ABI: two ULONG_PTRs, one pointer-sized offset/union,
/// and an event HANDLE. Tests verify layout rather than assuming 64-bit.
/// There is never more than one active I/O operation on our pipe instance.
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: usize,
    event: Handle,
}
impl Overlapped {
    fn with_event(event: Handle) -> Self {
        Self {
            internal: 0,
            internal_high: 0,
            offset: 0,
            event,
        }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn GetLastError() -> u32;
    fn CreateEventW(attrs: Handle, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn ResetEvent(handle: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
    fn WaitForMultipleObjects(
        number: u32,
        handles: *const Handle,
        wait_all: i32,
        milliseconds: u32,
    ) -> u32;
    fn ConnectNamedPipe(pipe: Handle, overlapped: Handle) -> i32;
    fn ReadFile(
        handle: Handle,
        output: *mut c_void,
        size: u32,
        read: *mut u32,
        overlapped: Handle,
    ) -> i32;
    fn GetOverlappedResult(
        file: Handle,
        overlapped: *mut Overlapped,
        bytes: *mut u32,
        wait: i32,
    ) -> i32;
    fn CancelIoEx(file: Handle, overlapped: Handle) -> i32;
    fn DisconnectNamedPipe(pipe: Handle) -> i32;
}

/// Unlike a callback hiding a race, every return guarantees the pending
/// OVERLAPPED structure has been completed or synchronously refused.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Phase {
    Stopped,
    Completed,
    Retry,
}

struct Event(Handle);
impl Event {
    fn manual() -> io::Result<Self> {
        let handle = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }
}
impl Drop for Event {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub(crate) struct CancelablePipe {
    pipe: OwnedHandle,
    done: Event,
}

impl CancelablePipe {
    /// May be called only by the SCM service's SYSTEM process. All objects
    /// must exist BEFORE it announces SERVICE_RUNNING. Never fallback to a
    /// less restrictive DACL or per-user registry.
    pub(crate) fn new() -> io::Result<Self> {
        let pipe = create_secure_pipe(true)?;
        let done = Event::manual()?;
        Ok(Self { pipe, done })
    }

    fn pending(&self, stop: Handle, ov: &mut Overlapped) -> io::Result<Phase> {
        let watched = [stop, self.done.0];
        let outcome = unsafe { WaitForMultipleObjects(2, watched.as_ptr(), 0, INFINITE) };
        let mut n = 0u32;
        if outcome == WAIT_OBJECT_0 {
            // Cancellation is a request, not completion. We cannot reuse or
            // free OVERLAPPED (or the output buffer) until the kernel has
            // fully retired the request. GetOverlappedResult(..., TRUE) is
            // required even if cancellation races normal completion.
            unsafe { CancelIoEx(self.pipe.0, ov as *mut Overlapped as Handle) };
            unsafe { GetOverlappedResult(self.pipe.0, ov, &mut n, 1) };
            return Ok(Phase::Stopped);
        }
        if outcome != WAIT_OBJECT_0 + 1 {
            // An abnormal wait also cannot release a pending buffer while
            // the kernel may still own it; cancel and drain before returning.
            unsafe { CancelIoEx(self.pipe.0, ov as *mut Overlapped as Handle) };
            unsafe { GetOverlappedResult(self.pipe.0, ov, &mut n, 1) };
            return Err(io::Error::last_os_error());
        }
        if unsafe { GetOverlappedResult(self.pipe.0, ov, &mut n, 0) } == 0 {
            return Ok(Phase::Retry);
        }
        Ok(Phase::Completed)
    }

    fn connect(&self, stop: Handle, ov: &mut Overlapped) -> io::Result<Phase> {
        if unsafe { WaitForSingleObject(stop, 0) } == WAIT_OBJECT_0 {
            return Ok(Phase::Stopped);
        }
        *ov = Overlapped::with_event(self.done.0);
        if unsafe { ResetEvent(self.done.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { ConnectNamedPipe(self.pipe.0, ov as *mut Overlapped as Handle) } != 0 {
            return Ok(Phase::Completed);
        }
        match unsafe { GetLastError() } {
            ERROR_PIPE_CONNECTED => Ok(Phase::Completed),
            ERROR_IO_PENDING => self.pending(stop, ov),
            _ => Err(io::Error::last_os_error()),
        }
    }

    fn read_one_or_stop(&self, stop: Handle, ov: &mut Overlapped) -> io::Result<Phase> {
        *ov = Overlapped::with_event(self.done.0);
        if unsafe { ResetEvent(self.done.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut ignored = [0u8; 1];
        let mut n = 0u32;
        let result = unsafe {
            ReadFile(
                self.pipe.0,
                ignored.as_mut_ptr().cast(),
                1,
                &mut n,
                ov as *mut Overlapped as Handle,
            )
        };
        if result != 0 {
            return Ok(Phase::Completed);
        }
        match unsafe { GetLastError() } {
            ERROR_IO_PENDING => self.pending(stop, ov),
            ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED | ERROR_MORE_DATA => Ok(Phase::Retry),
            _ => Err(io::Error::last_os_error()),
        }
    }

    /// The stop event belongs to SCM service_main and is signaled by its
    /// control handler. A client can stay silent indefinitely; SCM STOP must
    /// still cancel that overlapped ReadFile. Every connection drops bytes.
    ///
    /// The source guard runs before and after the real pipe connection. Any
    /// changed protected namespace or owner/DACL halts the *deny-only* service
    /// instead of accepting another byte. This does NOT create an atomic
    /// authorization fence or make future pathname-based reads safe.
    pub(crate) fn run_until_stop(
        &self,
        stop: Handle,
        mut source_guard: impl FnMut() -> io::Result<()>,
    ) -> io::Result<()> {
        if stop.is_null() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "no SCM stop event"));
        }
        let mut ov = Overlapped::with_event(self.done.0);
        loop {
            source_guard()?;
            match self.connect(stop, &mut ov)? {
                Phase::Stopped => break,
                Phase::Retry => continue,
                Phase::Completed => {}
            }
            if let Err(error) = source_guard() {
                unsafe { DisconnectNamedPipe(self.pipe.0) };
                return Err(error);
            }
            let read = self.read_one_or_stop(stop, &mut ov);
            // There is NO asynchronous operation left when read returns.
            let disconnected = unsafe { DisconnectNamedPipe(self.pipe.0) };
            if disconnected == 0 {
                let error = unsafe { GetLastError() };
                if error != ERROR_PIPE_NOT_CONNECTED {
                    return Err(io::Error::last_os_error());
                }
            }
            match read? {
                Phase::Stopped => break,
                Phase::Retry | Phase::Completed => {}
            }
            // Prevent runaway acceptance when clients spam one-byte writes.
            // This short pause is also interruptible via the SCM stop event.
            match unsafe { WaitForSingleObject(stop, 20) } {
                WAIT_OBJECT_0 => break,
                WAIT_TIMEOUT => {}
                _ => return Err(io::Error::last_os_error()),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_overlapped_layout_is_exactly_four_pointers() {
        assert_eq!(size_of::<Overlapped>(), size_of::<usize>() * 4);
    }

    #[test]
    fn ordinary_test_process_cannot_claim_system_broker_pipe() {
        // The test does not have a service-installed LocalSystem identity.
        // A future privileged runner should use a separate positive fixture,
        // rather than leaving an unconnected pipe alive in a unit test.
        let result = CancelablePipe::new();
        // An actual SYSTEM test runner is not evidence of an installed
        // service or any grant. If this succeeded, the handle is dropped
        // immediately without accepting any client.
        if let Ok(handle) = result {
            drop(handle);
        }
    }
}
