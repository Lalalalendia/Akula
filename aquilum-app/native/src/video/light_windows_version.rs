// VIDEO-18b-01: compatibility gate for the pinned official yt-dlp.exe.
// This OS check is not a protected installer, signed bundle, or tamper proof.
pub(super) fn supports_pinned_windows_video_runtime(major: u32) -> bool {
    major >= 10
}

pub(crate) fn require_pinned_windows_video_runtime() -> Result<(), String> {
    // The ABI layout exactly matches RTL_OSVERSIONINFOW / OSVERSIONINFOW.
    // RtlGetVersion returns the actual Windows version independent of
    // GetVersionExW application compatibility manifest behavior.
    #[allow(dead_code)]
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(version: *mut OsVersionInfo) -> i32;
    }
    let mut version = OsVersionInfo {
        size: std::mem::size_of::<OsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform: 0,
        service_pack: [0; 128],
    };
    // STATUS_SUCCESS == 0. Never start a sidecar on a failed version probe.
    if unsafe { RtlGetVersion(&mut version) } != 0 {
        return Err("VIDEO-18b cannot verify Windows version; pinned yt-dlp must not run".into());
    }
    if !supports_pinned_windows_video_runtime(version.major) {
        return Err(format!(
            "VIDEO-18b pinned yt-dlp requires Windows 10 or newer; found {}.{}",
            version.major, version.minor
        ));
    }
    Ok(())
}
