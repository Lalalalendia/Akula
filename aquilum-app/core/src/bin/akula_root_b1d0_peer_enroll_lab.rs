//! Non-shipping root-only one-time enrollment for dedicated local IPC UID.
//! No useradd, SSH, sudo, arbitrary path, listener or install authority.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1D0_PEER_ENROLL_HOLD");
        std::process::exit(2);
    }
    match aquilum_core::server_root_v3_trust::admin_enroll_first_peer_uid_fixed_root_once() {
        Ok(()) => println!("AKULA_B1D0_PEER_ENROLLED_PREVIEW_ONLY"),
        Err(_) => {
            eprintln!("AKULA_B1D0_PEER_ENROLL_HOLD");
            std::process::exit(2);
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1D0_PEER_ENROLL_HOLD");
    std::process::exit(2);
}
