//! Operator-only direct LAN proof: OS-protected Akula key -> real Iroh QUIC.
//! No file reading, staging, File Gate or background Sync.
use akula_staya_integration::os_peer::OsPinnedEndpoint;
use iroh::{EndpointAddr, EndpointId};
use std::{env, net::SocketAddr, str::FromStr, time::Duration};

fn usage() -> String {
    "usage: akula-p2p-ping listen PEER_ID | connect PEER_ID IP:UDP_PORT".into()
}

async fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mode = args.next().ok_or_else(usage)?;
    let selected = args.next().ok_or_else(usage)?;
    let address = args.next();
    if args.next().is_some()
        || (mode == "listen" && address.is_some())
        || (mode == "connect" && address.is_none())
        || (mode != "listen" && mode != "connect")
    {
        return Err(usage());
    }
    // Does not auto-enroll. Both users must explicitly confirm the full peer ID.
    let endpoint = OsPinnedEndpoint::bind().await?;
    println!("LOCAL_ENDPOINT_ID={}", endpoint.id());
    for socket in endpoint.bound_sockets() {
        println!("BOUND_UDP={socket} (use this port with a reachable LAN IP)");
    }

    let result = if mode == "listen" {
        let session = endpoint.accept_selected(&selected).await?;
        tokio::time::timeout(Duration::from_secs(20), async {
            let (mut tx, mut rx) = session
                .authorize_new_operation()?
                .accept_bi()
                .await
                .map_err(|error| format!("accept stream: {error}"))?;
            if rx
                .read_to_end(32)
                .await
                .map_err(|error| error.to_string())?
                != b"AKULA_HELLO_V1"
            {
                return Err("unexpected greeting".to_owned());
            }
            tx.write_all(b"AKULA_OK_V1")
                .await
                .map_err(|error| error.to_string())?;
            tx.finish().map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "message timeout".to_owned())?
    } else {
        let remote_id = EndpointId::from_str(&selected)
            .map_err(|_| "invalid full peer EndpointId".to_owned())?;
        let socket: SocketAddr = address
            .expect("validated address")
            .parse()
            .map_err(|_| "invalid IP:UDP_PORT".to_owned())?;
        let session = endpoint
            .connect_selected(&selected, EndpointAddr::new(remote_id).with_ip_addr(socket)
            .await?;
        tokio::time::timeout(Duration::from_secs(20), async {
            let (mut tx, mut rx) = session
                .authorize_new_operation()?
                .open_bi()
                .await
                .map_err(|error| format!("open stream: {error}"))?;
            tx.write_all(b"AKULA_HELLO_V1")
                .await
                .map_err(|error| error.to_string())?;
            tx.finish().map_err(|error| error.to_string())?;
            if rx.read_to_end(32).await.map_err(|error| error.to_string())?
                != b"AKULA_OK_V1"
            {
                return Err("unexpected reply".to_owned());
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "message timeout".to_owned())?
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
