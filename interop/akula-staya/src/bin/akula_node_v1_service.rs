//! Standalone non-root Node Iroh binary. Shipping requires independently
//! signed root approval and a deployed, immutable executable under /opt.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("AKULA_NODE_V1_HOLD: Linux only");
    std::process::exit(78);
}

#[cfg(target_os = "linux")]
#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    if std::env::args().collect::<Vec<_>>().as_slice().len() != 2
        || std::env::args().nth(1).as_deref() != Some("--serve-iroh")
    {
        eprintln!("AKULA_NODE_V1_HOLD: fixed operation required");
        std::process::exit(78);
    }
    if let Err(reason) =
        akula_staya_integration::node_v1_runtime::service::run_provisioned_iroh().await
    {
        eprintln!("AKULA_NODE_V1_HOLD: {reason}");
        std::process::exit(78);
    }
}
