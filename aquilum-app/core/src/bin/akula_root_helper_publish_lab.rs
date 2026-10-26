//! Non-shipping, one-shot administrator entry for B1-E2E-08b.
//! No CLI arguments, SSH, sudo, environment-provided paths or keys.
//! The independently authenticated administrator binary and production
//! root-trust enrollment must already exist; this command cannot create them.
//! It only publishes an already verified signed helper as mode 0500 HOLD.
#[cfg(target_os = "linux")]
fn main() {
    if std::env::args_os().len() != 1 {
        eprintln!("AKULA_B1D0_ADMIN_HELPER_PUBLICATION_REFUSED");
        std::process::exit(2);
    }
    let receipt =
        aquilum_core::server_root_v3_trust::helper_install::admin_install_fixed_root_helper_once();
    if let Ok(receipt) = receipt {
        if receipt.remains_hold && receipt.size >= 64 && receipt.sha256 != [0; 32] {
            println!("AKULA_B1D0_ADMIN_HELPER_FILE_PUBLISHED_HOLD");
            return;
        }
    }
    eprintln!("AKULA_B1D0_ADMIN_HELPER_PUBLICATION_REFUSED");
    std::process::exit(2);
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_B1D0_ADMIN_HELPER_PUBLICATION_REFUSED");
    std::process::exit(2);
}
