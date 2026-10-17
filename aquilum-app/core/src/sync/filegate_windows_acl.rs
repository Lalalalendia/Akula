//! Limited, read-only Windows OWNER + DACL audit on PINNED NT handles.
//!
//! Refuse missing/null DACLs, foreign owners, unrecognized ACE types and
//! write/delete/DACL-control grants to principals other than this token,
//! SYSTEM or BUILTIN\Administrators. This is NOT effective-access analysis,
//! immutable namespace custody, authenticated VaultId or writer admission.
//! Same-user processes, privileges and ancestor ACLs remain outside proof.

#![cfg(windows)]

use super::VaultPortError;
use std::ffi::c_void;
use std::fs::File;
use std::mem::{size_of, size_of_val};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;

const SE_FILE_OBJECT: u32 = 1;
const OWNER_AND_DACL: u32 = 0x0000_0001 | 0x0000_0004;
const TOKEN_QUERY: u32 = 0x0000_0008;
const TOKEN_USER_CLASS: u32 = 1;
const TOKEN_OWNER_CLASS: u32 = 4;
const ACL_SIZE_INFORMATION: u32 = 2;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const INHERIT_ONLY_ACE: u8 = 0x08;
const SYSTEM_SID: u32 = 22; // WinLocalSystemSid
const ADMINISTRATORS_SID: u32 = 26; // WinBuiltinAdministratorsSid

// Includes folder create, write/append, delete child, delete, WRITE_DAC,
// WRITE_OWNER, and GENERIC_WRITE/ALL. All auditing is conservative and
// deliberately rejects unrecognized ACE forms instead of guessing grants.
const NAMESPACE_WRITE: u32 =
    0x0000_0002 | 0x0000_0004 | 0x0000_0010 | 0x0000_0040
    | 0x0000_0100 | 0x0001_0000 | 0x0004_0000 | 0x0008_0000
    | 0x1000_0000 | 0x4000_0000;

#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct TokenUser {
    user: SidAndAttributes,
}

#[repr(C)]
struct TokenOwner {
    owner: *mut c_void,
}

#[repr(C)]
#[derive(Default)]
struct AclSizeInfo {
    ace_count: u32,
    bytes_in_use: u32,
    bytes_free: u32,
}

#[repr(C)]
struct AceHeader {
    ace_type: u8,
    ace_flags: u8,
    ace_size: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[link(name = "advapi32")]
extern "system" {
    fn OpenProcessToken(process: *mut c_void, access: u32, token: *mut *mut c_void) -> i32;
    fn GetTokenInformation(
        token: *mut c_void,
        class: u32,
        info: *mut c_void,
        size: u32,
        needed: *mut u32,
    ) -> i32;
    fn GetSecurityInfo(
        object: *mut c_void,
        object_type: u32,
        security_information: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut c_void,
        sacl: *mut *mut c_void,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn GetAclInformation(
        acl: *mut c_void,
        info: *mut c_void,
        info_len: u32,
        class: u32,
    ) -> i32;
    fn GetAce(acl: *mut c_void, index: u32, entry: *mut *mut c_void) -> i32;
    fn EqualSid(left: *mut c_void, right: *mut c_void) -> i32;
    fn IsValidSid(sid: *mut c_void) -> i32;
    fn GetLengthSid(sid: *mut c_void) -> u32;
    fn CreateWellKnownSid(
        sid_type: u32,
        domain_sid: *mut c_void,
        result: *mut c_void,
        size: *mut u32,
    ) -> i32;
}

struct OwnedSecurityDescriptor(*mut c_void);
impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: GetSecurityInfo allocated this descriptor via LocalAlloc
        // and this is its only owner. NULL is accepted by LocalFree.
        unsafe { LocalFree(self.0); }
    }
}

fn refuse() -> VaultPortError {
    VaultPortError::PermissionDenied
}

fn os_error(context: &str) -> VaultPortError {
    VaultPortError::Io(format!("{context}: {}", std::io::Error::last_os_error()))
}

fn well_known_sid(kind: u32) -> Result<Vec<u64>, VaultPortError> {
    let mut buffer = vec![0u64; 16];
    let mut len = size_of_val(buffer.as_slice()) as u32;
    // SAFETY: aligned, writable buffer and in/out size remain valid.
    let ok = unsafe {
        CreateWellKnownSid(kind, ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut len)
    };
    if ok == 0 || len < 8 || len as usize > size_of_val(buffer.as_slice()) {
        return Err(os_error("CreateWellKnownSid"));
    }
    Ok(buffer)
}

/// A negative gate on the opened file or directory HANDLE, not its name.
/// No file/ACL is changed. A successful result still cannot grant a write.
pub(super) fn audit_owner_dacl(file: &File) -> Result<(), VaultPortError> {
    let mut token_raw = ptr::null_mut();
    // SAFETY: process pseudo-handle is live and token_raw is an out-pointer.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token_raw) } == 0
        || token_raw.is_null()
    {
        return Err(os_error("OpenProcessToken"));
    }
    // SAFETY: OpenProcessToken transferred ownership of a unique HANDLE.
    let token = unsafe { OwnedHandle::from_raw_handle(token_raw) };
    let mut needed = 0u32;
    // Expected first call: return 0 and set required token buffer size.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(), TOKEN_USER_CLASS,
            ptr::null_mut(), 0, &mut needed,
        );
    }
    if needed < size_of::<TokenUser>() as u32 || needed > 4096 {
        return Err(refuse());
    }
    let mut data = vec![0u64; (needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: aligned output covers the requested TokenUser and variable SID.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TOKEN_USER_CLASS,
            data.as_mut_ptr().cast(),
            size_of_val(data.as_slice()) as u32,
            &mut needed,
        )
    } == 0
    {
        return Err(os_error("GetTokenInformation"));
    }
    // SAFETY: successful TokenUser query initializes its fixed-size header.
    let user_sid = unsafe { (*(data.as_ptr().cast::<TokenUser>())).user.sid };
    if user_sid.is_null() || unsafe { IsValidSid(user_sid) } == 0 {
        return Err(refuse());
    }

    // In elevated tokens a newly created directory may be owned by the
    // token's DEFAULT owner group (e.g. BUILTIN\\Administrators), not the
    // TokenUser SID. Both identifiers must come from this live token.
    let mut owner_needed = 0u32;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(), TOKEN_OWNER_CLASS,
            ptr::null_mut(), 0, &mut owner_needed,
        );
    }
    if owner_needed < size_of::<TokenOwner>() as u32 || owner_needed > 4096 {
        return Err(refuse());
    }
    let mut owner_data =
        vec![0u64; (owner_needed as usize).div_ceil(size_of::<u64>())];
    // SAFETY: the aligned buffer contains the fixed TOKEN_OWNER pointer and
    // its SID data only after GetTokenInformation succeeds.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(), TOKEN_OWNER_CLASS,
            owner_data.as_mut_ptr().cast(),
            size_of_val(owner_data.as_slice()) as u32,
            &mut owner_needed,
        )
    } == 0
    {
        return Err(os_error("GetTokenInformation(TokenOwner)"));
    }
    // SAFETY: initialized TOKEN_OWNER layout has its Owner pointer at offset 0.
    let default_owner = unsafe { (*(owner_data.as_ptr().cast::<TokenOwner>())).owner };
    if default_owner.is_null() || unsafe { IsValidSid(default_owner) } == 0 {
        return Err(refuse());
    }

    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: file handle is live and was opened for READ_CONTROL. The
    // returned owner and ACL pointers belong to the descriptor allocation.
    let status = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_FILE_OBJECT, OWNER_AND_DACL,
            &mut owner, ptr::null_mut(), &mut dacl, ptr::null_mut(),
            &mut descriptor,
        )
    };
    let _allocation = OwnedSecurityDescriptor(descriptor);
    if status != 0 {
        return Err(VaultPortError::Io(format!("GetSecurityInfo error {status}")));
    }
    // NULL DACL grants unrestricted access; a missing owner cannot be bound.
    if descriptor.is_null() || owner.is_null() || dacl.is_null()
        || unsafe { IsValidSid(owner) } == 0
        || (unsafe { EqualSid(owner, user_sid) } == 0
            && unsafe { EqualSid(owner, default_owner) } == 0)
    {
        return Err(refuse());
    }
    let mut info = AclSizeInfo::default();
    // SAFETY: DACL points into the live owned security descriptor.
    if unsafe {
        GetAclInformation(
            dacl, (&mut info as *mut AclSizeInfo).cast(),
            size_of::<AclSizeInfo>() as u32, ACL_SIZE_INFORMATION,
        )
    } == 0
    {
        return Err(os_error("GetAclInformation"));
    }
    if info.ace_count > 4096 || info.bytes_in_use < 8 {
        return Err(refuse());
    }
    let system_sid = well_known_sid(SYSTEM_SID)?;
    let admins_sid = well_known_sid(ADMINISTRATORS_SID)?;
    let system = system_sid.as_ptr() as *mut c_void;
    let admins = admins_sid.as_ptr() as *mut c_void;

    for index in 0..info.ace_count {
        let mut ace = ptr::null_mut();
        // SAFETY: GetAce returns a pointer within the live DACL, or fails.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 || ace.is_null() {
            return Err(os_error("GetAce"));
        }
        // SAFETY: A successful GetAce returns an ACE header of at least four
        // bytes. For allow ACEs we additionally check the minimum SID length.
        let header = unsafe { &*(ace.cast::<AceHeader>()) };
        if header.ace_size < 4 || u32::from(header.ace_size) > info.bytes_in_use {
            return Err(refuse());
        }
        if header.ace_flags & INHERIT_ONLY_ACE != 0 {
            // Inherit-only ACEs do not grant access to THIS opened object.
            // A child lock is separately audited through its own HANDLE.
            continue;
        }
        if header.ace_type == ACCESS_DENIED_ACE_TYPE {
            continue; // A deny ACE never adds permission.
        }
        if header.ace_type != ACCESS_ALLOWED_ACE_TYPE || header.ace_size < 16 {
            // Object/callback ACE layouts are not interpreted: fail closed.
            return Err(refuse());
        }
        // An effective inherited allow ACE is treated like an explicit
        // allow ACE; only strictly inherit-only entries are skipped above.
        // SAFETY: ACCESS_ALLOWED_ACE layout is header(4) mask(4) SID(>=8).
        let mask = unsafe { *ace.cast::<u8>().add(4).cast::<u32>() };
        if mask & NAMESPACE_WRITE == 0 {
            continue;
        }
        let granted_sid: *mut c_void = unsafe { ace.cast::<u8>().add(8).cast() };
        // SAFETY: only a validated ACCESS_ALLOWED_ACE containing the SID is
        // inspected; verify both SID validity and length bounded by ACE.
        if unsafe { IsValidSid(granted_sid) } == 0 {
            return Err(refuse());
        }
        let sid_size = unsafe { GetLengthSid(granted_sid) } as usize;
        if sid_size < 8 || sid_size + 8 > usize::from(header.ace_size) {
            return Err(refuse());
        }
        // SYSTEM/Administrators are *not* a defense against elevated or
        // hostile same-user actors. This is only a restrictive ACL signal.
        if unsafe { EqualSid(granted_sid, user_sid) } == 0
            && unsafe { EqualSid(granted_sid, system) } == 0
            && unsafe { EqualSid(granted_sid, admins) } == 0
        {
            return Err(refuse());
        }
    }

    Ok(())
}
