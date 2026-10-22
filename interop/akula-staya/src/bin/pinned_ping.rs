//! Manual LAN-only peer test using Akula's existing protected Iroh identity.
//! No file selection, GET, File Gate, auto-pairing, relay, or installed startup.
use akula_staya_integration::live_identity::{
    require_pinned_live_peer, with_protected_iroh_secret,
};
use aquilum_core::p2p_device_identity::peer_pin::{self, PeerState};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, endpoint::presets};
use std::{env, net::SocketAddr, str::FromStr, time::Duration};

const ALPN: &[u8] = b"akula/p2p/lan-ping/1";
const MESSAGE: &[u8] = b"AKULA_P2P_HELLO_V1";
const REPLY: &[u8] = b"AKULA_P2P_OK_V1";

enum Mode {
    Listen(String),
    Connect(String, SocketAddr),
}

fn usage() -> String {
    "usage: akula-p2p-ping listen PEER_ID | connect PEER_ID IP:UDP_PORT".into()
}

fn full_id(value: String) -> Result<String, String> {
    if value.len() != 64
        || value.bytes().all(|byte| byte == b'0')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("expected the complete lowercase 64-digit Iroh EndpointId".into());
    }
    Ok(value)
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Mode, String> {
    let action = args.next().ok_or_else(usage)?;
    let selected = full_id(args.next().ok_or_else(usage)?)?;
    let address = args.next();
    if args.next().is_some() {
        return Err(usage());
    }
    match (action.as_str(), address) {
        ("listen", None) => Ok(Mode::Listen(selected)),
        ("connect", Some(text)) => {
            let socket: SocketAddr = text.parse().map_err(|_| "invalid IP:UDP_PORT")?;
            if socket.port() == 0 || socket.ip().is_unspecified() {
                return Err("a reachable nonzero destination UDP socket is required".into());
            }
            Ok(Mode::Connect(selected, socket))
        }
        _ => Err(usage()),
    }
}

fn check_user_pin(selected: &str) -> Result<(), String> {
    match peer_pin::peer_state(selected) {
        Ok(PeerState::Confirmed) => Ok(()),
        Ok(PeerState::Revoked) => Err("peer is revoked".into()),
        Ok(PeerState::NotFound) => Err("peer was not independently confirmed".into()),
        Err(error) => Err(format!("protected peer pin: {error}")),
    }
}

async fn listen(endpoint: &Endpoint, selected: &str) -> Result<(), String> {
    let incoming = tokio::time::timeout(Duration::from_secs(60), endpoint.accept())
        .await
        .map_err(|_| "waiting for the selected peer timed out")?
        .ok_or("QUIC endpoint stopped")?;
    let connecting = incoming
        .accept()
        .map_err(|error| format!("incoming QUIC: {error}"))?;
    let connection = tokio::time::timeout(Duration::from_secs(15), connecting)
        .await
        .map_err(|_| "QUIC handshake timed out")?
        .map_err(|error| format!("QUIC handshake: {error}"))?;
    require_pinned_live_peer(&connection, selected).map_err(|error| {
        connection.close(403u32.into(), b"peer not OS-pinned");
        format!("actual remote_id denied: {error}")
    })?;
    let action = tokio::time::timeout(Duration::from_secs(15), async {
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(|error| format!("read stream: {error}"))?;
        require_pinned_live_peer(&connection, selected)
            .map_err(|error| format!("new operation denied: {error}"))?;
        if recv.read_to_end(64).await.map_err(|error| error.to_string())? != MESSAGE {
            return Err("unexpected peer greeting".into());
        }
        send.write_all(REPLY).await.map_err(|error| error.to_string())?;
        send.finish().map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "request timed out")?;
    connection.close(0u8.into(), b"LAN diagnostic complete");
    action
}

async fn connect(endpoint: &Endpoint, selected: &str, socket: SocketAddr) -> Result<(), String> {
    let remote = EndpointId::from_str(selected)
        .map_err(|_| "selected ID is not an Iroh endpoint key")?;
    let destination = EndpointAddr::new(remote).with_ip_addr(socket);
    let connection = tokio::time::timeout(
        Duration::from_secs(15),
        endpoint.connect(destination, ALPN),
    )
    .await
    .map_err(|_| "direct QUIC connection timed out")?
    .map_err(|error| format!("direct QUIC: {error}"))?;
    require_pinned_live_peer(&connection, selected).map_err(|error| {
        connection.close(403u32.into(), b"peer not OS-pinned");
        format!("actual remote_id denied: {error}")
    })?;
    let action = tokio::time::timeout(Duration::from_secs(15), async {
        require_pinned_live_peer(&connection, selected)
            .map_err(|error| format!("new operation denied: {error}"))?;
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|error| format!("open stream: {error}"))?;
        send.write_all(MESSAGE)
            .await
            .map_err(|error| error.to_string())?;
        send.finish().map_err(|error| error.to_string())?;
        if recv.read_to_end(64).await.map_err(|error| error.to_string())? != REPLY {
            return Err("unexpected peer reply".into());
        }
        Ok::<(), String>(())
    })
    .await
    .map_err(|_| "request timed out")?;
    connection.close(0u8.into(), b"LAN diagnostic complete");
    action
}

async fn run() -> Result<(), String> {
    let mode = parse(env::args().skip(1))?;
    let selected = match &mode {
        Mode::Listen(value) | Mode::Connect(value, _) => value,
    };
    // This preflight is only an early refusal. The real identity is checked
    // again from authenticated QUIC remote_id() before each new operation.
    check_user_pin(selected)?;
    let key = with_protected_iroh_secret(|secret| secret)
        .map_err(|error| format!("local protected Iroh key: {error}"))?;
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(key)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .map_err(|error| format!("bind direct-only Iroh endpoint: {error}"))?;

    println!("LOCAL_ENDPOINT_ID={}", endpoint.id());
    for socket in endpoint.bound_sockets() {
        println!("BOUND_UDP={socket} (choose your reachable LAN IP)");
    }
    let result = match mode {
        Mode::Listen(peer) => listen(&endpoint, &peer).await,
        Mode::Connect(peer, socket) => connect(&endpoint, &peer, socket).await,
    };
    endpoint.close().await;
    result?;
    println!("PINNED_QUIC_OK=true FILE_TRANSFER=false FILE_GATE=false");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("P2P_PINNED_QUIC_ERROR={error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(items: &[&str]) -> Result<Mode, String> {
        parse(items.iter().map(|item| (*item).to_owned()))
    }

    #[test]
    fn accepts_only_explicit_full_peer_and_direct_socket() {
        let id = "12".repeat(32);
        assert!(matches!(arguments(&["listen", &id]), Ok(Mode::Listen(_))));
        assert!(matches!(
            arguments(&["connect", &id, "192.168.1.5:12345"]),
            Ok(Mode::Connect(_, _))
        ));
        assert!(arguments(&["connect", &id, "0.0.0.0:80"]).is_err());
        assert!(arguments(&["connect", &id, "127.0.0.1:0"]).is_err());
        assert!(arguments(&["listen", &id, "unexpected"]).is_err());
        assert!(arguments(&["listen", &"A".repeat(64)]).is_err());
        assert!(arguments(&["listen", &"0".repeat(64)]).is_err());
        assert!(arguments(&["listen", "short"]).is_err());
    }
}
