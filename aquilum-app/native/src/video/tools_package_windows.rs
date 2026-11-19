//! VIDEO-TOOLS-01e: native Windows read-only protection checks.
//! No executable path leaves the signed-package module. No launch authority.
//! The standalone lab witness (cfg below) is ONLY for disposable PR runners.
#![allow(non_snake_case)]

use std::ffi::{c_void, OsStr, OsString};
use std::fs;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}
const PROGRAM_FILES_X64: Guid = Guid {
    data1: 0x6d809377,
    data2: 0x6af0,
    data3: 0x444b,
    data4: [0x89, 0x57, 0xa3, 0x77, 0x3f, 0x02, 0x20, 0x0e],
};
#[repr(C)]
struct OsVersionInfoW {
    size: u32,
    major: u32,
    minor: u32,
    build: u32,
    platform: u32,
    service_pack: [u16; 128],
}
#[repr(C)]
struct Acl {
    revision: u8,
    reserved: u8,
    size: u16,
    count: u16,
    reserved2: u16,
}
#[repr(C)]
struct AceHeader {
    kind: u8,
    flags: u8,
    size: u16,
}
struct ComMemory(*mut u16);
impl Drop for ComMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CoTaskMemFree(self.0.cast()) };
        }
    }
}
struct LocalMemory(*mut c_void);
impl Drop for LocalMemory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { LocalFree(self.0) };
        }
    }
}

#[link(name = "shell32")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(
        id: *const Guid,
        flags: u32,
        token: *mut c_void,
        path: *mut *mut u16,
    ) -> i32;
}
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoTaskMemFree(value: *mut c_void);
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn RtlGetVersion(version: *mut OsVersionInfoW) -> i32;
}
#[link(name = "advapi32")]
unsafe extern "system" {
    fn GetNamedSecurityInfoW(
        path: *const u16,
        object_type: u32,
        information: u32,
        owner: *mut *mut c_void,
        group: *mut *mut c_void,
        dacl: *mut *mut Acl,
        sacl: *mut *mut Acl,
        descriptor: *mut *mut c_void,
    ) -> u32;
    fn GetAce(acl: *mut Acl, index: u32, ace: *mut *mut c_void) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
}
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(block: *mut c_void) -> *mut c_void;
}

fn hold(message: &str) -> String {
    format!("VIDEO_TOOLS_01E_HOLD: {message}")
}
fn utf16_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}
fn utf16_result(ptr: *const u16, max: usize) -> Result<Vec<u16>, String> {
    if ptr.is_null() {
        return Err(hold("Windows API returned a null UTF-16 string"));
    }
    let mut result = Vec::new();
    for index in 0..max {
        let value = unsafe { *ptr.add(index) };
        if value == 0 {
            return Ok(result);
        }
        result.push(value);
    }
    Err(hold("Windows API returned an unbounded UTF-16 string"))
}
fn sid_text(sid: *mut c_void) -> Result<String, String> {
    if sid.is_null() {
        return Err(hold("missing security principal"));
    }
    let mut value = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut value) } == 0 {
        return Err(hold("could not decode Windows principal SID"));
    }
    let owned = LocalMemory(value.cast());
    String::from_utf16(&utf16_result(owned.0.cast(), 256)?)
        .map_err(|_| hold("principal SID was not UTF-16"))
}
fn privileged_sid(sid: &str) -> bool {
    matches!(
        sid,
        "S-1-5-18"
            | "S-1-5-32-544"
            | "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
    )
}
pub(super) fn known_program_files_x64() -> Result<PathBuf, String> {
    if std::mem::size_of::<usize>() != 8 {
        return Err(hold("x64 process required"));
    }
    let mut version = OsVersionInfoW {
        size: std::mem::size_of::<OsVersionInfoW>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    if unsafe { RtlGetVersion(&mut version) } != 0 || version.major < 10 {
        return Err(hold("Windows 10 or later required"));
    }
    let mut raw_path = ptr::null_mut();
    let status = unsafe {
        SHGetKnownFolderPath(&PROGRAM_FILES_X64, 0, ptr::null_mut(), &mut raw_path)
    };
    let owned = ComMemory(raw_path);
    if status != 0 {
        return Err(hold("Windows system-known Program Files lookup failed"));
    }
    let value = OsString::from_wide(&utf16_result(owned.0, 32768)?);
    let root = PathBuf::from(value);
    if !root.is_absolute() {
        return Err(hold("OS-known Program Files was not absolute"));
    }
    check_ancestors(&root)?;
    Ok(root)
}
fn check_ancestors(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(hold("relative or ambiguous Windows path"));
    }
    for ancestor in path.ancestors() {
        let info = fs::symlink_metadata(ancestor)
            .map_err(|_| hold("protected path ancestor unavailable"))?;
        if !info.is_dir() || info.file_attributes() & 0x400 != 0 {
            return Err(hold("junction, reparse or non-directory ancestor"));
        }
    }
    Ok(())
}
/// Only a fixed, publisher-owned product directory can be discovered.
/// This must not be replaced by a PATH, ProgramFiles environment or caller path.
pub(super) fn installed_root() -> Result<Option<PathBuf>, String> {
    let root = known_program_files_x64()?.join("Akula Video Tools");
    if !root.exists() {
        return Ok(None);
    }
    check_ancestors(&root)?;
    Ok(Some(root))
}
fn check_object_acl(path: &Path) -> Result<(), String> {
    let wide = utf16_null(path.as_os_str());
    let mut owner: *mut c_void = ptr::null_mut();
    let mut dacl: *mut Acl = ptr::null_mut();
    let mut raw: *mut c_void = ptr::null_mut();
    // SE_FILE_OBJECT = 1; OWNER_SECURITY_INFORMATION = 1;
    // DACL_SECURITY_INFORMATION = 4. Named lookup is read-only.
    let code = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            1,
            1 | 4,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut raw,
        )
    };
    let descriptor = LocalMemory(raw);
    if code != 0 || descriptor.0.is_null() {
        return Err(hold("unable to read Windows owner and DACL"));
    }
    if !privileged_sid(&sid_text(owner)?) {
        return Err(hold("installed package has an untrusted owner"));
    }
    if dacl.is_null() {
        return Err(hold("installed package has an unrestricted null DACL"));
    }
    let acl = unsafe { &*dacl };
    if acl.size < 8 || acl.count > 256 {
        return Err(hold("invalid or excessive Windows ACL"));
    }
    // WriteData/AppendData/WriteEA/DeleteChild/WriteAttributes/Delete/
    // WriteDac/WriteOwner plus GENERIC_WRITE, GENERIC_ALL and MAXIMUM_ALLOWED.
    const MUTATION_RIGHTS: u32 = 0x0000_0002
        | 0x0000_0004
        | 0x0000_0010
        | 0x0000_0040
        | 0x0000_0100
        | 0x0001_0000
        | 0x0004_0000
        | 0x0008_0000
        | 0x0200_0000
        | 0x1000_0000
        | 0x4000_0000;
    for index in 0..u32::from(acl.count) {
        let mut ace = ptr::null_mut();
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 || ace.is_null() {
            return Err(hold("cannot inspect installed ACL entry"));
        }
        let header = unsafe { &*(ace as *const AceHeader) };
        // ACE_INHERIT_ONLY does not apply to this object; each installed
        // child file's effective inherited ACL is checked independently.
        if header.flags & 0x08 != 0 {
            continue;
        }
        if header.kind == 1 {
            continue; // explicit ACCESS_DENIED_ACE cannot create access
        }
        if header.kind != 0 || header.size < 12 {
            return Err(hold("unsupported or malformed Windows ACL entry"));
        }
        let mask = unsafe { *((ace as *const u8).add(4) as *const u32) };
        let sid = unsafe { (ace as *mut u8).add(8) }.cast();
        if mask & MUTATION_RIGHTS != 0 && !privileged_sid(&sid_text(sid)?) {
            return Err(hold("non-elevated trustee can mutate protected package"));
        }
    }
    Ok(())
}
/// Checks exact package identity, all child file ACLs and parent reparse state.
/// Does not return any executable handle/path or authorize launching it.
pub(super) fn check_installed_acl(root: &Path, expected: &[&str]) -> Result<(), String> {
    if expected.len() < 4 || expected.len() > 6 {
        return Err(hold("unexpected number of installed package files"));
    }
    check_ancestors(root)?;
    let objects = fs::read_dir(root).map_err(|_| hold("cannot enumerate package root"))?;
    let mut seen = Vec::new();
    for item in objects {
        let item = item.map_err(|_| hold("cannot enumerate installed object"))?;
        let name = item.file_name();
        if !expected.iter().any(|n| name == OsStr::new(n))
            || seen.iter().any(|n: &OsString| n == &name)
        {
            return Err(hold("extra or duplicate installed package object"));
        }
        let meta = fs::symlink_metadata(item.path())
            .map_err(|_| hold("cannot inspect installed file"))?;
        if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {
            return Err(hold("junction, reparse or non-file package object"));
        }
        check_object_acl(&item.path())?;
        seen.push(name);
    }
    if seen.len() != expected.len() {
        return Err(hold("signed package is incomplete"));
    }
    check_object_acl(root)?;
    check_ancestors(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_known_root_is_absolute_on_supported_windows() {
        let root = known_program_files_x64().expect("Windows API must supply protected x64 root");
        assert!(root.is_absolute());
        assert!(root.join("Akula Video Tools").starts_with(&root));
    }
    #[test]
    fn arbitrary_temp_directory_is_not_a_complete_signed_install() {
        let tmp = std::env::temp_dir();
        assert!(check_installed_acl(&tmp, &[
            "akula-video-mux.exe",
            "akula-yt-dlp.exe",
            "akula-quickjs.exe",
            "uninstall.exe"
        ]).is_err());
    }
    #[test]
    fn genuine_protected_nsis_fixture_has_strict_native_acl() {
        if let Some(root) = std::env::var_os("AKULA_VIDEO_TOOLS_01E_LAB_ROOT") {
            let root = PathBuf::from(root);
            check_installed_acl(&root, &[
                "akula-video-mux.exe",
                "akula-yt-dlp.exe",
                "akula-quickjs.exe",
                "uninstall.exe",
            ])
            .expect("genuine installed PE ACL");
            println!("VIDEO_TOOLS_01E_NATIVE_ACL_PASS root=1 files=4 release=0");
        }
    }
}

#[cfg(akula_video_tools_acl_witness)]
fn main() {
    let root = std::env::var_os("AKULA_VIDEO_TOOLS_01E_LAB_ROOT")
        .expect("VIDEO_TOOLS_01E_HOLD: PR-only disposable root missing");
    let status = check_installed_acl(
        Path::new(&root),
        &[
            "akula-video-mux.exe",
            "akula-yt-dlp.exe",
            "akula-quickjs.exe",
            "uninstall.exe",
        ],
    );
    match status {
        Ok(()) => println!("VIDEO_TOOLS_01E_NATIVE_ACL_PASS root=1 files=4 release=0"),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(3);
        }
    }
}
