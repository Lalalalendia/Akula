//! Non-shipping root-admin one-shot initial release approval.
//! No SSH, IPC, privilege escalation, Node installation or systemd activation.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1A_FIRST_RELEASE_ENROLL_HOLD");
        std::process::exit(2);
    }
    match aquilum_core::server_root_v3_trust::admin_enroll_first_b1a_fixed_root_once() {
        Ok(()) => println!("AKULA_B1A_FIRST_RELEASE_ENROLLED_HOLD"),
        Err(_) => {
            eprintln!("AKULA_B1A_FIRST_RELEASE_ENROLL_HOLD");
            std::process::exit(2);
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1A_FIRST_RELEASE_ENROLL_HOLD");
    std::process::exit(2);
}
