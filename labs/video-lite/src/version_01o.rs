//! VIDEO-TOOLS-01o: source-controlled, compile-time LAB identity only.
//! Proves that two independently built Windows PEs are not merely signed
//! manifests with different version labels. Never an approved product update.
#[cfg(feature = "lab_mux_020")]
const VERSION: &str = "0.2.0";
#[cfg(not(feature = "lab_mux_020"))]
const VERSION: &str = "0.1.0";

pub(crate) fn probe_identity() -> bool {
    let mut args = std::env::args_os();
    let _program = args.next();
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--video-tools-lab-build-id"))
        || args.next().is_some()
    {
        return false;
    }
    println!("AKULA_VIDEO_TOOLS_LAB_MUX_VERSION={VERSION}");
    true
}
