//! Explicit operator-only OS-key QUIC proof for two independent Windows PCs.
//! Direct IP or separately selected internet N0 discovery/relay, HELLO/OK only.
//! NO Staya GET, Vault, File Gate, user-file reads or background Sync.
//! Reuses the ONE merged live_identity seam; does not add a second transport.

use akula_staya_integration::live_identity::{
    require_pinned_live_peer, with_protected_iroh_secret,
};
use aquilum_core::p2p_device_identity::peer_pin;
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, endpoint::presets};
use std::{env, net::SocketAddr, str::FromStr, time::Duration};

const ALPN: &[u8] = b"akula/p2p-os-lan-proof/1";
const MESSAGE_TIMEOUT: Duration = Duration::from_secs(20);

fn usage() -> String {
    "usage: akula-p2p-lan listen PEER_ID | connect PEER_ID LAN_IP:UDP_PORT | internet-listen PEER_ID | internet-connect PEER_ID".into()
}

async fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mode = args.next().ok_or_else(usage)?;
    let selected = args.next().ok_or_else(usage)?;
    let socket_text = args.next();
    let internet = matches!(mode.as_str(), "internet-listen" | "internet-connect");
    let listen = matches!(mode.as_str(), "listen" | "internet-listen");
    let connect = matches!(mode.as_str(), "connect" | "internet-connect");
    if args.next().is_some()
        || (!listen && !connect)
        || (listen && socket_text.is_some())
        || (mode == "connect" && socket_text.is_none())
        || (mode == "internet-connect" && socket_text.is_some())
    {
        return Err(usage());
    }
    let peer_id = EndpointId::from_str(&selected)
        .map_err(|_| "expected a complete peer Iroh EndpointId".to_owned())?;

    // The expected key must be independently confirmed in Akula Core's
    // existing protected peer pin. A ticket is not a confirmation.
    peer_pin::require_pinned_remote(&selected, peer_id.as_bytes())
        .map_err(|error| format!("selected OS peer is not confirmed: {error}"))?;

    // This is the EXISTING #625 credential->Iroh seam. No seed in argv,
    // environment variables, YAML, plaintext files, or stdout.
    let secret = with_protected_iroh_secret(|secret| secret)
        .map_err(|error| format!("protected local Iroh key unavailable: {error}"))?;
    // The explicit internet-* modes are the ONLY ones that enable N0
    // infrastructure. Ordinary listen/connect remain direct with no relay,
    // discovery, NAT mapping or network metadata sent to a third party.
    let builder = if internet {
        Endpoint::builder(presets::N0)
    } else {
        Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
    };
    if internet {
        eprintln!(
            "INTERNET_OPT_IN=1: uses third-party n0 DNS/Pkarr discovery and              encrypted relay fallback; relay operators may see IP/timing metadata."
        );
    }
    let endpoint = builder
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|error| format!("QUIC bind: {error}"))?;
    println!("LOCAL_ENDPOINT_ID={}", endpoint.id());
    if internet {
        tokio::time::timeout(Duration::from_secs(60), endpoint.online())
            .await
            .map_err(|_| "no n0 relay/discovery connectivity within 60 seconds".to_owned())?;
        println!("INTERNET_DISCOVERY_READY=true PEER_ID_ONLY=true");
    } else {
        for socket in endpoint.bound_sockets() {
            println!("LISTEN_UDP={socket} (use actual reachable LAN IP, not wildcard)");
        }
    }
    let result = if listen {
        let incoming = tokio::time::timeout(Duration::from_secs(300), endpoint.accept())
            .await
            .map_err(|_| "waiting for the selected peer timed out".to_owned())?
            .ok_or_else(|| "local endpoint closed".to_owned())?;
        let handshake = incoming
            .accept()
            .map_err(|error| format!("incoming TLS accept: {error}"))?;
        let connection = tokio::time::timeout(MESSAGE_TIMEOUT, handshake)
            .await
            .map_err(|_| "incoming QUIC handshake timed out".to_owned())?
            .map_err(|error| format!("incoming QUIC handshake: {error}"))?;
        require_pinned_live_peer(&connection, &selected)
            .map_err(|error| format!("authenticated incoming peer denied: {error}"))?;
        let exchange = tokio::time::timeout(MESSAGE_TIMEOUT, async {
            let (mut tx, mut rx) = connection
                .accept_bi()
                .await
                .map_err(|error| format!("accept bounded greeting: {error}"))?;
            let message = rx
                .read_to_end(32)
                .await
                .map_err(|error| format!("read bounded greeting: {error}"))?;
            if message != b"AKULA_HELLO_V1" {
                return Err("unexpected greeting".to_owned());
            }
            // Check persistent revoke again before the final response.
            require_pinned_live_peer(&connection, &selected)
                .map_err(|error| format!("selected peer was revoked: {error}"))?;
            tx.write_all(b"AKULA_OK_V1")
                .await
                .map_err(|error| format!("write bounded reply: {error}"))?;
            tx.finish()
                .map_err(|error| format!("finish bounded reply: {error}"))?;
            // QUIC finish alone does not guarantee delivery before close.
            match tx.stopped().await {
                Ok(None) => {}
                Ok(Some(_)) => return Err("peer stopped the reply stream".to_owned()),
                Err(error) => return Err(format!("reply delivery not confirmed: {error}")),
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "bounded greeting/reply timed out".to_owned())?;
        connection.close(0u8.into(), b"probe complete");
        exchange
    } else {
        let address = if internet {
            // N0 address lookup resolves this already OS-confirmed EndpointId;
            // an untrusted DNS/Pkarr answer cannot change live QUIC identity.
            EndpointAddr::new(peer_id)
        } else {
            let socket: SocketAddr = socket_text
                .expect("validated connect address")
                .parse()
                .map_err(|_| "expected a numeric LAN_IP:UDP_PORT".to_owned())?;
            EndpointAddr::new(peer_id).with_ip_addr(socket)
        };
        let connect_timeout = if internet {
            Duration::from_secs(90)
        } else {
            MESSAGE_TIMEOUT
        };
        let connection = tokio::time::timeout(connect_timeout, endpoint.connect(address, ALPN))
            .await
            .map_err(|_| "direct QUIC connection timed out".to_owned())?
            .map_err(|error| format!("direct QUIC connection: {error}"))?;
        require_pinned_live_peer(&connection, &selected)
            .map_err(|error| format!("authenticated outgoing peer denied: {error}"))?;
        let exchange = tokio::time::timeout(MESSAGE_TIMEOUT, async {
            let (mut tx, mut rx) = connection
                .open_bi()
                .await
                .map_err(|error| format!("open greeting stream: {error}"))?;
            tx.write_all(b"AKULA_HELLO_V1")
                .await
                .map_err(|error| format!("send greeting: {error}"))?;
            tx.finish()
                .map_err(|error| format!("finish greeting: {error}"))?;
            let reply = rx
                .read_to_end(32)
                .await
                .map_err(|error| format!("read bounded reply: {error}"))?;
            if reply != b"AKULA_OK_V1" {
                return Err("unexpected authenticated reply".to_owned());
            }
            // Preserve the session until the already-sent greeting is
            // acknowledged; otherwise immediate connection close may drop it.
            match tx.stopped().await {
                Ok(None) => {}
                Ok(Some(_)) => return Err("peer stopped the greeting stream".to_owned()),
                Err(error) => return Err(format!("greeting delivery not confirmed: {error}")),
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "bounded greeting/reply timed out".to_owned())?;
        connection.close(0u8.into(), b"probe complete");
        exchange
    };

    endpoint.close().await;
    result?;
    println!(
        "OS_PINNED_QUIC_OK=true ROUTE_MODE={} FILE_TRANSFER=false SYNC=false",
        if internet { "INTERNET_N0" } else { "DIRECT_IP" }
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("AKULA_LAN_PROBE_ERROR={error}");
        std::process::exit(1);
    }
}
