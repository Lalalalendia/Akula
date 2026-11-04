//! Administrative, fixed-path one-time B0 v3 trust enrollment.
//! NOT a daemon, network service, SSH request handler or auto-installer.
//! The administrator MUST independently verify this binary and the root-owned
//! candidate policy before invoking it with already granted UID0 authority.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B0V3_ADMIN_ENROLL_REFUSED");
        std::process::exit(2);
    }
    match aquilum_core::server_root_v3_trust::admin_enroll_fixed_root_once() {
        Ok(()) => println!("AKULA_B0V3_ADMIN_ENROLLED_HOLD"),
        Err(_) => {
            eprintln!("AKULA_B0V3_ADMIN_ENROLL_REFUSED");
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B0V3_ADMIN_ENROLL_REFUSED");
    std::process::exit(2);
}
