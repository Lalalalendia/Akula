//! Explicit root-operator one-shot empty /opt/akula-node authorization.
//! It cannot be reached through Akula UI, SSH or IPC, and never executes Node.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1D0_OPT_ENROLL_HOLD");
        std::process::exit(2);
    }
    match aquilum_core::server_root_v3_trust::admin_enroll_fixed_opt_once() {
        Ok(()) => println!("AKULA_B1D0_OPT_ENROLLED_EMPTY_HOLD"),
        Err(_) => {
            eprintln!("AKULA_B1D0_OPT_ENROLL_HOLD");
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1D0_OPT_ENROLL_HOLD");
    std::process::exit(2);
}
