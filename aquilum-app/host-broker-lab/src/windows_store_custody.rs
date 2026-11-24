//! RN-4E1b-03c1a: pinned, read-only Windows Broker custody preflight.
//!
//! No appdata, no automatic provisioning, no repairs. A real SCM service may
//! enter RUNNING only if a pre-existing separate installed state marker AND
//! its parent directory have SYSTEM/Administrators OWNER, protected non-null
//! DACL, a dedicated service SID ACE, no unknown ACE and no other write ACE.
//!
//! This proof returns only opened HANDLEs. It grants NO peer access or file
//! authority. The marker contains no approved devices; the real durable
//! policy store and WAL/SHM fencing are separately blocked.
//!
//! The policy marker path is fixed for this Windows CI research binary.
//! C:\ProgramData and installer ownership of its full ancestor chain,
//! signed executable, service upgrades and administrator hostility are NOT
//! proven. Never enable this module from installed native Akula.

#![cfg(windows)]

use std::ffi::c_void;
use std::io::{self, ErrorKind};
use std::mem::{size_of, size_of_val};
use std::os::windows::ffi::OsStrExt;
use std::ptr;

type Handle = *mut c_void;
const INVALID_HANDLE: Handle = -1isize as Handle;
const SERVICE_SID_ACCOUNT: &str = r"NT SERVICE\AkulaHostBrokerResearch";
const CUSTODY_DIR: &str = r"C:\ProgramData\AkulaHostBrokerResearch";
const CUSTODY_MARKER: &str = r"C:\ProgramData\AkulaHostBrokerResearch\authority.marker";

const FILE_READ_ATTRIBUTES: u32 = 0x80;
const READ_CONTROL: u32 = 0x0002_0000;
const FILE_SHARE_READ: u32 = 0x1;
const FILE_SHARE_WRITE: u32 = 0x2;
// No FILE_SHARE_DELETE limits some mutation paths but does NOT guarantee
// a directory name cannot change while HANDLEs remain open on Windows.
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const SE_FILE_OBJECT: u32 = 1;
const OWNER_DACL: u32 = 0x1 | 0x4;
const SE_DACL_PROTECTED: u16 = 0x1000;
const ACL_SIZE_INFORMATION: u32 = 2;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const INHERIT_ONLY_ACE: u8 = 0x08;
const WIN_LOCAL_SYSTEM_SID: u32 = 22;
const WIN_BUILTIN_ADMIN_SID: u32 = 26;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const NAMESPACE_WRITE: u32 = 0x0000_0002
    | 0x0000_0004
    | 0x0000_0010
    | 0x0000_0040
    | 0x0000_0100
    | 0x0001_0000
    | 0x0004_0000
    | 0x0008_0000
    | 0x1000_0000
    | 0x4000_0000;

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[repr(C)]
#[derive(Default)]
struct ByHandleInformation {
    attributes: u32,
    created: FileTime,
    accessed: FileTime,
    written: FileTime,
    volume_serial: u32,
    // Exact Win32 BY_HANDLE_FILE_INFORMATION ABI: filesize words precede
    // nNumberOfLinks. Mixing this order reports size_high as link_count.
    size_high: u32,
    size_low: u32,
    link_count: u32,
    file_index_high: u32,
    file_index_low: u32,
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
    kind: u8,
    flags: u8,
    size: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        path: *const u16,
        rights: u32,
        share: u32,
        attrs: Handle,
        disposition: u32,
        flags: u32,
        template: Handle,
    ) -> Handle;
    fn CloseHandle(handle: Handle) -> i32;
    fn GetLastError() -> u32;
    fn GetFileInformationByHandle(handle: Handle, data: *mut ByHandleInformation) -> i32;
    fn LocalFree(memory: Handle) -> Handle;
}

#[link(name = "advapi32")]
extern "system" {
    fn GetSecurityInfo(
        handle: Handle,
        kind: u32,
        requested: u32,
        owner: *mut Handle,
        group: *mut Handle,
        dacl: *mut Handle,
        sacl: *mut Handle,
        descriptor: *mut Handle,
    ) -> u32;
    fn GetSecurityDescriptorControl(
        descriptor: Handle,
        control: *mut u16,
        revision: *mut u32,
    ) -> i32;
    fn GetAclInformation(acl: Handle, info: Handle, len: u32, class: u32) -> i32;
    fn GetAce(acl: Handle, index: u32, entry: *mut Handle) -> i32;
    fn EqualSid(first: Handle, second: Handle) -> i32;
    fn IsValidSid(sid: Handle) -> i32;
    fn GetLengthSid(sid: Handle) -> u32;
    fn CreateWellKnownSid(kind: u32, domain: Handle, out: Handle, length: *mut u32) -> i32;
    fn LookupAccountNameW(
        machine: *const u16,
        account: *const u16,
        sid: Handle,
        sid_len: *mut u32,
        domain: *mut u16,
        domain_len: *mut u32,
        kind: *mut u32,
    ) -> i32;
}

fn refuse(msg: &'static str) -> io::Error {
    io::Error::new(ErrorKind::PermissionDenied, msg)
}
fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
fn sid_equal(a: Handle, b: Handle) -> bool {
    (unsafe { EqualSid(a, b) }) != 0
}
struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct OwnedDescriptor(Handle);
impl Drop for OwnedDescriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn well_known_sid(kind: u32) -> io::Result<Vec<usize>> {
    let mut data = vec![0usize; 16];
    let mut len = size_of_val(data.as_slice()) as u32;
    if unsafe { CreateWellKnownSid(kind, ptr::null_mut(), data.as_mut_ptr().cast(), &mut len) } == 0
        || len < 8
        || len as usize > size_of_val(data.as_slice())
    {
        return Err(refuse("well-known Windows SID could not be resolved"));
    }
    Ok(data)
}

fn broker_service_sid() -> io::Result<Vec<usize>> {
    let name = wide(SERVICE_SID_ACCOUNT);
    let (mut sid_size, mut domain_size, mut sid_kind) = (0, 0, 0);
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            name.as_ptr(),
            ptr::null_mut(),
            &mut sid_size,
            ptr::null_mut(),
            &mut domain_size,
            &mut sid_kind,
        );
    }
    if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
        || sid_size < 8
        || sid_size > 1024
        || domain_size > 1024
    {
        return Err(refuse("uninstalled service SID is not an authority"));
    }
    let mut sid = vec![0usize; (sid_size as usize).div_ceil(size_of::<usize>())];
    let mut domain = vec![0u16; (domain_size as usize).max(1)];
    if unsafe {
        LookupAccountNameW(
            ptr::null(),
            name.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_size,
            domain.as_mut_ptr(),
            &mut domain_size,
            &mut sid_kind,
        )
    } == 0
        || unsafe { IsValidSid(sid.as_mut_ptr().cast()) } == 0
    {
        return Err(refuse("the expected service SID was not resolved"));
    }
    Ok(sid)
}

fn opened_non_reparse(path: &str, directory: bool) -> io::Result<OwnedHandle> {
    let flags = FILE_FLAG_OPEN_REPARSE_POINT
        | if directory {
            FILE_FLAG_BACKUP_SEMANTICS
        } else {
            0
        };
    let raw = unsafe {
        CreateFileW(
            wide(path).as_ptr(),
            FILE_READ_ATTRIBUTES | READ_CONTROL,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null_mut(),
            OPEN_EXISTING,
            flags,
            ptr::null_mut(),
        )
    };
    if raw.is_null() || raw == INVALID_HANDLE {
        return Err(io::Error::last_os_error());
    }
    let file = OwnedHandle(raw);
    let mut info = ByHandleInformation::default();
    if unsafe { GetFileInformationByHandle(file.0, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || (info.attributes & FILE_ATTRIBUTE_DIRECTORY != 0) != directory
        || info.file_index_high == 0 && info.file_index_low == 0
        || info.volume_serial == 0
        || (!directory && info.link_count != 1)
    {
        return Err(refuse(
            "unexpected reparse link, object type or file identity",
        ));
    }
    Ok(file)
}

fn strict_protected_acl(
    handle: &OwnedHandle,
    system: Handle,
    admins: Handle,
    broker: Handle,
) -> io::Result<()> {
    let (mut owner, mut dacl, mut desc) = (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    let status = unsafe {
        GetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            OWNER_DACL,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut desc,
        )
    };
    let _allocation = OwnedDescriptor(desc);
    if status != 0 || desc.is_null() || owner.is_null() || dacl.is_null() {
        return Err(refuse("missing protected owner/DACL"));
    }
    if unsafe { IsValidSid(owner) } == 0 || !(sid_equal(owner, system) || sid_equal(owner, admins))
    {
        return Err(refuse("host policy owner must be SYSTEM or Administrators"));
    }
    let (mut flags, mut revision) = (0u16, 0u32);
    if unsafe { GetSecurityDescriptorControl(desc, &mut flags, &mut revision) } == 0
        || flags & SE_DACL_PROTECTED == 0
    {
        return Err(refuse("host policy DACL inherits mutable ancestor policy"));
    }
    let mut info = AclSizeInfo::default();
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut AclSizeInfo).cast(),
            size_of::<AclSizeInfo>() as u32,
            ACL_SIZE_INFORMATION,
        )
    } == 0
        || info.ace_count > 128
    {
        return Err(refuse("invalid host policy access control list"));
    }
    let mut broker_write = false;
    let mut system_write = false;
    for n in 0..info.ace_count {
        let mut ptr_ace = ptr::null_mut();
        if unsafe { GetAce(dacl, n, &mut ptr_ace) } == 0 || ptr_ace.is_null() {
            return Err(refuse("invalid ACE pointer"));
        }
        let header = unsafe { &*(ptr_ace.cast::<AceHeader>()) };
        if header.kind != ACCESS_ALLOWED_ACE_TYPE && header.kind != ACCESS_DENIED_ACE_TYPE {
            return Err(refuse("unrecognized or conditional host policy ACE"));
        }
        if header.size < 16 {
            return Err(refuse("short host policy ACE"));
        }
        let raw = ptr_ace.cast::<u8>();
        let mask = unsafe { *(raw.add(4).cast::<u32>()) };
        let who = unsafe { raw.add(8).cast::<c_void>() };
        if unsafe { IsValidSid(who) } == 0
            || unsafe { GetLengthSid(who) } as usize > header.size as usize - 8
        {
            return Err(refuse("malformed host policy access SID"));
        }
        if header.kind == ACCESS_ALLOWED_ACE_TYPE && mask & NAMESPACE_WRITE != 0 {
            // An INHERIT_ONLY ACE is not effective for this handle.
            // Untrusted inherited write ACEs still cause refusal.
            if header.flags & INHERIT_ONLY_ACE == 0 {
                if sid_equal(who, broker) {
                    broker_write = true;
                }
                if sid_equal(who, system) {
                    system_write = true;
                }
            }
            if !sid_equal(who, broker) && !sid_equal(who, system) && !sid_equal(who, admins) {
                return Err(refuse("ordinary user can modify broker-owned authority"));
            }
        }
    }
    if !broker_write || !system_write {
        return Err(refuse(
            "missing SYSTEM or distinct Broker service-SID access",
        ));
    }
    Ok(())
}

/// Owned opened-handle witness, never a proof of policy content or a permit.
/// Windows can rename an opened directory despite no FILE_SHARE_DELETE:
/// these HANDLEs pin objects, NOT their original pathname. Admin remains TCB.
/// A short-lived comparison is not a substitute for handle-relative I/O.
/// Only a production signed issuer with protected root enrollment can grant.
pub(crate) struct CustodyWitness {
    _directory: OwnedHandle,
    _marker: OwnedHandle,
}

fn opened_identity(handle: &OwnedHandle) -> io::Result<(u32, u32, u32)> {
    let mut info = ByHandleInformation::default();
    if unsafe { GetFileInformationByHandle(handle.0, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.volume_serial == 0 || (info.file_index_high == 0 && info.file_index_low == 0) {
        return Err(refuse("missing by-HANDLE physical identity"));
    }
    Ok((
        info.volume_serial,
        info.file_index_high,
        info.file_index_low,
    ))
}

impl CustodyWitness {
    pub(crate) fn inspect_fixed() -> io::Result<Self> {
        Self::inspect_at(CUSTODY_DIR, CUSTODY_MARKER)
    }

    /// Reopen BOTH exact fixed names, revalidate their current OS owner/DACL
    /// and compare fresh HANDLE identities against the startup-pinned ones.
    /// Any missing/renamed/replaced name or changed ACL aborts a deny-only IPC
    /// iteration. This cannot protect a future pathname use after the check.
    pub(crate) fn recheck_fixed_names(&self) -> io::Result<()> {
        let fresh = Self::inspect_fixed()?;
        if opened_identity(&fresh._directory)? != opened_identity(&self._directory)?
            || opened_identity(&fresh._marker)? != opened_identity(&self._marker)?
        {
            return Err(refuse(
                "fixed Broker namespace no longer names the pinned objects",
            ));
        }
        Ok(())
    }
    fn inspect_at(directory: &str, marker: &str) -> io::Result<Self> {
        if !directory.starts_with(r"C:\")
            || marker != format!(r"{}\authority.marker", directory)
            || directory.len() <= 3
        {
            return Err(refuse("unexpected host installer policy namespace"));
        }
        let system = well_known_sid(WIN_LOCAL_SYSTEM_SID)?;
        let admins = well_known_sid(WIN_BUILTIN_ADMIN_SID)?;
        let service = broker_service_sid()?;
        let s = system.as_ptr() as Handle;
        let a = admins.as_ptr() as Handle;
        let b = service.as_ptr() as Handle;
        let dir = opened_non_reparse(directory, true)?;
        strict_protected_acl(&dir, s, a, b)?;
        let file = opened_non_reparse(marker, false)?;
        strict_protected_acl(&file, s, a, b)?;
        Ok(Self {
            _directory: dir,
            _marker: file,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn win32_by_handle_file_information_has_exact_field_offsets() {
        // The SDK layout is 52 bytes, with DWORD volume serial at 28,
        // file size high/low at 32/36 and hardlink count at 40.
        assert_eq!(std::mem::size_of::<ByHandleInformation>(), 52);
        assert_eq!(std::mem::offset_of!(ByHandleInformation, volume_serial), 28);
        assert_eq!(std::mem::offset_of!(ByHandleInformation, size_high), 32);
        assert_eq!(std::mem::offset_of!(ByHandleInformation, size_low), 36);
        assert_eq!(std::mem::offset_of!(ByHandleInformation, link_count), 40);
        assert_eq!(
            std::mem::offset_of!(ByHandleInformation, file_index_high),
            44
        );
        assert_eq!(
            std::mem::offset_of!(ByHandleInformation, file_index_low),
            48
        );
    }

    #[test]
    fn held_directory_handles_do_not_certify_a_stable_path() {
        use std::fs;
        let parent = std::env::temp_dir().join(format!(
            "akai-custody-path-falsifier-{}",
            std::process::id()
        ));
        if parent.exists() {
            panic!("refuse to mutate preexisting path");
        }
        fs::create_dir(&parent).unwrap();
        let original = parent.join("original");
        let renamed = parent.join("renamed");
        fs::create_dir(&original).unwrap();
        let pinned = opened_non_reparse(&original.to_string_lossy(), true).unwrap();
        let id = opened_identity(&pinned).unwrap();
        if fs::rename(&original, &renamed).is_ok() {
            // The namespace changed, even though the old object still exists.
            assert!(opened_non_reparse(&original.to_string_lossy(), true).is_err());
            let moved = opened_non_reparse(&renamed.to_string_lossy(), true).unwrap();
            assert_eq!(opened_identity(&moved).unwrap(), id);
            drop(moved);
            fs::create_dir(&original).unwrap();
            let replacement = opened_non_reparse(&original.to_string_lossy(), true).unwrap();
            assert_ne!(opened_identity(&replacement).unwrap(), id);
            drop(replacement);
        } else {
            let same = opened_non_reparse(&original.to_string_lossy(), true).unwrap();
            assert_eq!(opened_identity(&same).unwrap(), id);
            drop(same);
        }
        drop(pinned);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn absent_installer_store_is_never_auto_created() {
        let target =
            std::env::temp_dir().join(format!("akula-broker-uninstalled-{}", std::process::id()));
        let before = target.exists();
        assert!(CustodyWitness::inspect_at(
            &target.to_string_lossy(),
            &target.join("authority.marker").to_string_lossy(),
        )
        .is_err());
        assert_eq!(target.exists(), before);
    }

    #[test]
    fn no_runtime_fallback_to_arbitrary_appdata_paths() {
        assert!(CustodyWitness::inspect_at(
            r"C:\Users\ordinary\AppData",
            r"C:\Users\ordinary\AppData\authority.marker",
        )
        .is_err());
        assert!(CustodyWitness::inspect_at("", "").is_err());
    }
}

/// Read-only diagnostic; no handle, token or permit escapes to the caller.
pub fn inspect_fixed_without_authorization() -> io::Result<()> {
    CustodyWitness::inspect_fixed().map(|_| ())
}
