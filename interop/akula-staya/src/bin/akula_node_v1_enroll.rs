//! One-shot, non-root enrollment called ONLY after signed installer approval.
//! Never installs packages, elevates privileges or enables a systemd unit.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_NODE_V1_HOLD: Linux only");
    std::process::exit(78);
}

#[cfg(target_os = "linux")]
fn main() {
    let mut args = std::env::args();
    let valid = args.next().is_some()
        && args.next().as_deref() == Some("--enroll-identities")
        && args.next().is_none();
    if !valid {
        eprintln!("AKULA_NODE_V1_HOLD: fixed enrollment operation required");
        std::process::exit(78);
    }
    if let Err(reason) = akula_staya_integration::node_v1_runtime::service::enroll_for_installer() {
        eprintln!("AKULA_NODE_V1_HOLD: {reason}");
        std::process::exit(78);
    }
}
