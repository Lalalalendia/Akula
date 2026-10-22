//! Optional OS-backed identity diagnostics. Never bundled in Akula GUI.
use aquilum_core::p2p_device_identity;

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if args.next().is_some() {
        eprintln!("usage: akula-p2p-identity --init | --id");
        std::process::exit(2);
    }
    let identity = match command.as_deref() {
        Some("--init") => p2p_device_identity::initialize(),
        Some("--id") => p2p_device_identity::load(),
        _ => {
            eprintln!("usage: akula-p2p-identity --init | --id");
            std::process::exit(2);
        }
    };
    match identity {
        Ok(identity) => println!("IROH_ENDPOINT_PUBLIC_HEX={}", identity.endpoint_id_hex()),
        Err(error) => {
            eprintln!("P2P_IDENTITY_ERROR={error}");
            std::process::exit(1);
        }
    }
}
