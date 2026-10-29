//! Protected Windows 7-compatible NTFS DACL for plaintext recovery snapshots.
//! Only the current user, SYSTEM and Administrators may read or modify them.
//! Does not defend against other processes running as the same user.
use std::ffi::{c_void, OsStr};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::{mem, ptr, slice};

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(token: *mut c_void, class: u32, buffer: *mut c_void, capacity: u32, needed: *mut u32) -> i32;
    fn ConvertSidToStringSidW(sid: *const c_void, text: *mut *mut u16) -> i32;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        sddl: *const u16, revision: u32, output: *mut *mut c_void, size: *mut u32,
    ) -> i32;
    fn GetSecurityDescriptorDacl(
        descriptor: *const c_void, present: *mut i32, dacl: *mut *mut c_void, defaulted: *mut i32,
    ) -> i32;
    fn SetNamedSecurityInfoW(
        path: *mut u16, kind: u32, information: u32, owner: *mut c_void, group: *mut c_void,
        dacl: *mut c_void, sacl: *mut c_void,
    ) -> u32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[repr(C)]
struct SidAndAttributes { sid: *mut c_void, attributes: u32 }

struct Token(*mut c_void);
impl Drop for Token { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
struct Local(*mut c_void);
impl Drop for Local { fn drop(&mut self) { unsafe { LocalFree(self.0); } } }

fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(std::iter::once(0)).collect()
}
fn current_sid() -> io::Result<String> {
    let mut handle = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), 0x0008, &mut handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = Token(handle);
    let mut needed = 0u32;
    unsafe { GetTokenInformation(token.0, 1, ptr::null_mut(), 0, &mut needed); }
    if needed < mem::size_of::<SidAndAttributes>() as u32 || needed > 65536 {
        return Err(io::Error::other("invalid Windows token user length"));
    }
    let mut data = vec![0usize; (needed as usize).div_ceil(mem::size_of::<usize>())];
    if unsafe { GetTokenInformation(token.0, 1, data.as_mut_ptr().cast(), needed, &mut needed) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*(data.as_ptr() as *const SidAndAttributes) };
    if user.sid.is_null() { return Err(io::Error::other("missing token SID")); }
    let mut text = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let allocation = Local(text.cast());
    let len = (0..256usize).find(|&index| unsafe { *text.add(index) } == 0)
        .ok_or_else(|| io::Error::other("unterminated token SID"))?;
    let sid = String::from_utf16(unsafe { slice::from_raw_parts(text, len) })
        .map_err(io::Error::other)?;
    drop(allocation);
    if !sid.starts_with("S-1-") || !sid.chars().all(|c| c.is_ascii_digit() || c == '-' || c == 'S') {
        return Err(io::Error::other("invalid token SID"));
    }
    Ok(sid)
}

/// Apply a protected, inheritable DACL. Call only after the caller verified
/// the path is a regular file/directory, never a symlink or reparse point.
/// Any OS failure blocks snapshot staging/reading instead of silently falling
/// back to broad permissions inherited from --data-dir.
pub(super) fn restrict(path: &Path) -> io::Result<()> {
    let sid = current_sid()?;
    let sddl = format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{sid})");
    let wide_sddl = wide(OsStr::new(&sddl));
    let mut raw = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(wide_sddl.as_ptr(), 1, &mut raw, ptr::null_mut())
    } == 0 { return Err(io::Error::last_os_error()); }
    let descriptor = Local(raw);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl = ptr::null_mut();
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if present == 0 || acl.is_null() { return Err(io::Error::other("missing recovery DACL")); }
    let mut name = wide(path.as_os_str());
    // SE_FILE_OBJECT=1, DACL_SECURITY_INFORMATION=0x4,
    // PROTECTED_DACL_SECURITY_INFORMATION=0x80000000. Supported on Windows 7.
    let error = unsafe {
        SetNamedSecurityInfoW(name.as_mut_ptr(), 1, 0x8000_0004, ptr::null_mut(), ptr::null_mut(), acl, ptr::null_mut())
    };
    if error != 0 { return Err(io::Error::from_raw_os_error(error as i32)); }
    Ok(())
}
