//! One signed Node ELF with exactly two fixed, independently guarded modes.
//! --enroll-identities: one-shot, non-root, no existing keys or owner policy.
//! --serve-iroh: non-root, pre-enrolled exact keys and root-owned policy v3.
//! No arbitrary paths, network targets, shell commands or installer privileges.
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_NODE_V1_HOLD: Linux only");
    std::process::exit(78);
}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let mut args = std::env::args();
    let _program = args.next();
    let mode = args.next();
    if args.next().is_some() {
        eprintln!("AKULA_NODE_V1_HOLD: exactly one fixed mode required");
        std::process::exit(78);
    }
    let result = match mode.as_deref() {
        Some("--enroll-identities") => {
            akula_staya_integration::node_v1_runtime::service::enroll_for_installer()
        }
        Some("--serve-iroh") => {
            akula_staya_integration::node_v1_runtime::service::run_provisioned_iroh().await
        }
        _ => Err("unknown or missing fixed Node operation"),
    };
    if let Err(reason) = result {
        eprintln!("AKULA_NODE_V1_HOLD: {reason}");
        std::process::exit(78);
    }
}
