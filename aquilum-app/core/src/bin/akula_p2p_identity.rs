//! Optional operator identity and peer-confirmation diagnostics.
//! Never bundled into Akula GUI; these commands issue no file permissions.
use aquilum_core::p2p_device_identity::{self, peer_pin};
use std::io::{self, IsTerminal};

fn usage() -> ! {
    eprintln!(
        "usage: akula-p2p-identity --init | --id | --peer-confirm ID | --peer-status ID | --peer-revoke ID"
    );
    std::process::exit(2);
}

fn full_entry(message: &str) -> Result<String, String> {
    if !io::stdin().is_terminal() {
        return Err("peer changes require an interactive terminal".into());
    }
    eprintln!("{message}");
    let mut confirmed = String::new();
    io::stdin()
        .read_line(&mut confirmed)
        .map_err(|error| format!("cannot read confirmation: {error}"))?;
    Ok(confirmed.trim_end_matches(&['\r', '\n'][..]).to_owned())
}

fn run() -> Result<String, String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| usage());
    let argument = args.next();
    if args.next().is_some() {
        usage();
    }
    match (command.as_str(), argument) {
        ("--init", None) => p2p_device_identity::initialize()
            .map(|identity| identity.endpoint_id_hex())
            .map_err(|error| error.to_string()),
        ("--id", None) => p2p_device_identity::load()
            .map(|identity| identity.endpoint_id_hex())
            .map_err(|error| error.to_string()),
        ("--peer-confirm", Some(discovered)) => {
            let independently_verified = full_entry(
                "Independently verify and TYPE the complete 64-character peer EndpointId:",
            )?;
            peer_pin::confirm_peer(&discovered, &independently_verified)
                .map(|()| "PEER_CONFIRMED (transport identity only)".into())
                .map_err(|error| error.to_string())
        }
        ("--peer-revoke", Some(remote)) => {
            let typed = full_entry(
                "To deny future operations, TYPE: REVOKE followed by a space and the entire peer EndpointId:",
            )?;
            if typed != format!("REVOKE {remote}") {
                return Err("revocation confirmation did not match".into());
            }
            peer_pin::revoke_peer(&remote)
                .map(|()| "PEER_REVOKED".into())
                .map_err(|error| error.to_string())
        }
        ("--peer-status", Some(remote)) => peer_pin::peer_state(&remote)
            .map(|state| format!("PEER_STATE={state:?}"))
            .map_err(|error| error.to_string()),
        _ => usage(),
    }
}

fn main() {
    match run() {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("P2P_IDENTITY_ERROR={error}");
            std::process::exit(1);
        }
    }
}
