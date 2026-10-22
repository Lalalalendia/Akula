//! Explicit direct-only LAN diagnostic: real Akula OS key + live Iroh QUIC pin.
//! One bounded HELLO/OK. Never sends file bytes or issues File Gate grants.
use akula_staya_integration::live_identity::{
    require_pinned_live_peer, with_protected_iroh_secret,
};
use aquilum_core::p2p_device_identity::peer_pin;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, endpoint::presets};
use std::{env, net::SocketAddr, str::FromStr, time::Duration};

const ALPN: &[u8] = b"akula/staya/manual-lan-probe/1";
const HELLO: &[u8] = b"AKULA_P2P_HELLO_V1";
const REPLY: &[u8] = b"AKULA_P2P_OK_V1";
const USAGE: &str = "usage: akula-p2p-quic-probe listen PEER_ENDPOINT_ID | connect PEER_ENDPOINT_ID LAN_IP:UDP_PORT";

#[derive(Debug, Eq, PartialEq)]
enum Mode {
    Listen { peer: String },
    Connect { peer: String, address: SocketAddr },
}

fn parse_peer(text: &str) -> Result<EndpointId, String> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("peer ID must be exactly 64 lowercase hex characters".into());
    }
    let id = EndpointId::from_str(text).map_err(|_| "invalid Iroh EndpointId")?;
    if id.as_bytes() == &[0u8; 32] {
        return Err("zero peer ID is forbidden".into());
    }
    Ok(id)
}

fn parse_mode(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let args: Vec<String> = args.into_iter().collect();
    match args.as_slice() {
        [mode, peer] if mode == "listen" => {
            parse_peer(peer)?;
            Ok(Mode::Listen { peer: peer.clone() })
        }
        [mode, peer, address] if mode == "connect" => {
            parse_peer(peer)?;
            let address: SocketAddr = address
                .parse()
                .map_err(|_| "expected numeric LAN_IP:UDP_PORT, not a hostname")?;
            if address.port() == 0 {
                return Err("UDP port cannot be zero".into());
            }
            Ok(Mode::Connect {
                peer: peer.clone(),
                address,
            })
        }
        _ => Err(USAGE.to_owned()),
    }
}

fn preflight_peer(peer: &str) -> Result<EndpointId, String> {
    let id = parse_peer(peer)?;
    // Check actual OS store before opening any socket. No auto-enrollment.
    peer_pin::require_pinned_remote(peer, id.as_bytes())
        .map_err(|error| format!("peer not confirmed in protected store: {error}"))?;
    Ok(id)
}

fn require_live(connection: &iroh::endpoint::Connection, peer: &str) -> Result<(), String> {
    // Must read real TLS-authenticated remote_id and a fresh OS pin.
    require_pinned_live_peer(connection, peer)
        .map_err(|error| format!("real Iroh peer not admitted: {error}"))
}

async fn listen(endpoint: &Endpoint, peer: &str) -> Result<(), String> {
    let incoming = tokio::time::timeout(Duration::from_secs(180), endpoint.accept())
        .await
        .map_err(|_| "timed out waiting for the selected LAN peer")?
        .ok_or("Iroh endpoint stopped")?;
    let pending = incoming
        .accept()
        .map_err(|error| format!("incoming Iroh handshake: {error}"))?;
    let connection = tokio::time::timeout(Duration::from_secs(20), pending)
        .await
        .map_err(|_| "handshake timeout")?
        .map_err(|error| format!("authenticated handshake: {error}"))?;
    if let Err(error) = require_live(&connection, peer) {
        connection.close(403u32.into(), b"untrusted peer");
        return Err(error);
    }
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        // Revoked peers cannot start this new logical operation.
        require_live(&connection, peer)?;
        let (mut tx, mut rx) = connection
            .accept_bi()
            .await
            .map_err(|error| format!("accept stream: {error}"))?;
        let bytes = rx
            .read_to_end(64)
            .await
            .map_err(|error| format!("read bounded greeting: {error}"))?;
        if bytes != HELLO {
            return Err("invalid synthetic greeting".into());
        }
        tx.write_all(REPLY)
            .await
            .map_err(|error| format!("send synthetic reply: {error}"))?;
        tx.finish().map_err(|error| format!("finish reply: {error}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "probe operation timeout")?;
    connection.close(0u8.into(), b"probe complete");
    result
}

async fn connect(
    endpoint: &Endpoint,
    peer: &str,
    id: EndpointId,
    socket: SocketAddr,
) -> Result<(), String> {
    let addr = EndpointAddr::new(id).with_ip_addr(socket);
    let connection = tokio::time::timeout(Duration::from_secs(20), endpoint.connect(addr, ALPN))
        .await
        .map_err(|_| "direct LAN connect timeout")?
        .map_err(|error| format!("authenticated QUIC connect: {error}"))?;
    if let Err(error) = require_live(&connection, peer) {
        connection.close(403u32.into(), b"untrusted peer");
        return Err(error);
    }
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        require_live(&connection, peer)?;
        let (mut tx, mut rx) = connection
            .open_bi()
            .await
            .map_err(|error| format!("open stream: {error}"))?;
        tx.write_all(HELLO)
            .await
            .map_err(|error| format!("send bounded greeting: {error}"))?;
        tx.finish().map_err(|error| format!("finish greeting: {error}"))?;
        let reply = rx
            .read_to_end(64)
            .await
            .map_err(|error| format!("read reply: {error}"))?;
        if reply != REPLY {
            return Err("invalid synthetic reply".into());
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "probe operation timeout")?;
    connection.close(0u8.into(), b"probe complete");
    result
}

async fn run() -> Result<(), String> {
    let mode = parse_mode(env::args().skip(1))?;
    let selected = match &mode {
        Mode::Listen { peer } | Mode::Connect { peer, .. } => peer,
    };
    let selected_id = preflight_peer(selected)?;
    let secret = with_protected_iroh_secret(|key| key)
        .map_err(|error| format!("Akula OS-protected key unavailable: {error}"))?;
    if secret.public() == selected_id {
        return Err("self-connection is not a peer-pairing test".into());
    }
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .map_err(|error| format!("bind real direct QUIC: {error}"))?;
    println!("LOCAL_ENDPOINT_ID={}", endpoint.id());
    if matches!(&mode, Mode::Listen { .. }) {
        for socket in endpoint.bound_sockets() {
            println!("BOUND_UDP={socket} (use this port with the host LAN address)");
        }
    }
    let result = match mode {
        Mode::Listen { peer } => listen(&endpoint, &peer).await,
        Mode::Connect { peer, address } => connect(&endpoint, &peer, selected_id, address).await,
    };
    endpoint.close().await;
    result?;
    println!("PINNED_QUIC_OK=true FILE_TRANSFER=false FILE_GATE=false");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("P2P_LAN_PROBE_ERROR={error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_or_unexpected_commands_do_not_reach_os_credentials() {
        let id = iroh::SecretKey::generate().public().to_string();
        assert_eq!(
            parse_mode(["listen".into(), id.clone()]).unwrap(),
            Mode::Listen { peer: id.clone() }
        );
        assert!(parse_mode(["listen".into(), "0".repeat(64)]).is_err());
        assert!(parse_mode(["listen".into(), "A".repeat(64)]).is_err());
        assert!(parse_mode(["connect".into(), id.clone(), "peer.example:443".into()]).is_err());
        assert!(parse_mode(["connect".into(), id.clone(), "127.0.0.1:0".into()]).is_err());
        assert!(parse_mode(["listen".into(), id, "extra".into()]).is_err());
    }

    #[test]
    fn numeric_address_does_not_change_the_selected_peer_id() {
        let id = iroh::SecretKey::generate().public().to_string();
        assert_eq!(
            parse_mode(["connect".into(), id.clone(), "192.0.2.7:5050".into()]).unwrap(),
            Mode::Connect {
                peer: id,
                address: "192.0.2.7:5050".parse().unwrap(),
            }
        );
    }
}
