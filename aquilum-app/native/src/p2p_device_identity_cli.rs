//! Explicit developer/operator commands for the real protected device key.
//! No app startup creates or rotates an identity. No network/peer grant here.

use aquilum_core::p2p_device_identity;

pub(crate) fn dispatch() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    let command = args.next()?;
    let identity = match command.as_str() {
        "--p2p-device-init" => p2p_device_identity::initialize(),
        "--p2p-device-id" => p2p_device_identity::load(),
        _ => return None,
    };
    if args.next().is_some() {
        eprintln!("P2P_IDENTITY_ERROR=unexpected argument");
        return Some(2);
    }
    match identity {
        Ok(identity) => {
            // Public Ed25519 key only. Never serialize or log the private seed.
            println!("IROH_ENDPOINT_PUBLIC_HEX={}", identity.endpoint_id_hex());
            Some(0)
        }
        Err(error) => {
            eprintln!("P2P_IDENTITY_ERROR={error}");
            Some(1)
        }
    }
}
