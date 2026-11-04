//! Root-admin one-shot witness genesis for a clean Ubuntu/Debian VPS.
//! No SSH, client parameters, privilege escalation, Node execution, or
//! automatic repair. Binary distribution must be authenticated out of band.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1D0_WITNESS_INIT_HOLD");
        std::process::exit(2);
    }
    match aquilum_core::server_root_v3_trust::admin_initialize_first_root_witness() {
        Ok(()) => println!("AKULA_B1D0_WITNESS_INITIALIZED_HOLD"),
        Err(_) => {
            eprintln!("AKULA_B1D0_WITNESS_INIT_HOLD");
            std::process::exit(2);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1D0_WITNESS_INIT_HOLD");
    std::process::exit(2);
}
