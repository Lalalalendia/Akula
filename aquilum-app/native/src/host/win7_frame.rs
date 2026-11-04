// Windows 7 needs a native caption: borderless client-area geometry can
// overlap the system's non-client frame under Aero/DWM.
#[cfg(target_os = "windows")]
fn legacy_windows(major: u32, minor: u32) -> bool {
    major < 6 || (major == 6 && minor <= 1)
}

#[cfg(target_os = "windows")]
pub(crate) fn native_frame() -> bool {
    use std::sync::OnceLock;

    #[repr(C)]
    struct OsVersion {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }

    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OsVersion) -> i32;
    }

    static LEGACY: OnceLock<bool> = OnceLock::new();
    *LEGACY.get_or_init(|| {
        let mut version = OsVersion {
            size: std::mem::size_of::<OsVersion>() as u32,
            major: 0, minor: 0, build: 0, platform: 0, service_pack: [0; 128],
        };
        unsafe { RtlGetVersion(&mut version) == 0 && legacy_windows(version.major, version.minor) }
    })
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn native_frame() -> bool {
    cfg!(target_os = "macos")
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::legacy_windows;

    #[test]
    fn system_caption_is_reserved_for_legacy_windows() {
        assert!(legacy_windows(6, 0));
        assert!(legacy_windows(6, 1));
        assert!(!legacy_windows(6, 2));
        assert!(!legacy_windows(10, 0));
    }
}
