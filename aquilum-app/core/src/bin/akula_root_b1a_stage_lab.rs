//! Non-shipping one-shot UID0 staging of independently signed first B1a Node.
//! No SSH-supplied paths, API, install authority, sudo grant or systemd.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1A_FIRST_STAGE_HOLD");
        std::process::exit(2);
    }
    match aquilum_core::server_root_v3_trust::admin_stage_first_b1a_fixed_root_once() {
        Ok(()) => println!("AKULA_B1A_FIRST_SIGNED_STAGE_HOLD"),
        Err(_) => {
            eprintln!("AKULA_B1A_FIRST_STAGE_HOLD");
            std::process::exit(2);
        }
    }
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1A_FIRST_STAGE_HOLD");
    std::process::exit(2);
}
