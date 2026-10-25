//! Candidate for the fixed, separately signed root-owned B1d0 helper.
//! This is intentionally NON-SHIPPING and incapable of installation.
//! It accepts no command-line options, files, paths, SSH strings or keys.
//!
//! A future independently authorized systemd Accept=yes socket service can
//! supply one connected AF_UNIX stream as fd 0 (StandardInput=socket).
//! Today even a successfully authenticated owner PREVIEW exits nonzero HOLD.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1D0_ROOT_HELPER_HOLD");
        std::process::exit(2);
    }
    let accepted =
        aquilum_core::server_root_v3_trust::root_helper_entry::authenticated_owner_preview_once_fd0(
        );
    if accepted {
        println!("AKULA_B1D0_SIGNED_OWNER_PREVIEW_ONLY_INSTALL_HOLD");
    } else {
        eprintln!("AKULA_B1D0_ROOT_HELPER_HOLD");
    }
    // An authenticated preview is *never* a successful installation or an
    // instruction to execute a second program. Always fail closed until
    // independently authorized install dispatch is implemented and tested.
    std::process::exit(2);
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1D0_ROOT_HELPER_HOLD");
    std::process::exit(2);
}
