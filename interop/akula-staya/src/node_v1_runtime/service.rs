//! Explicit non-root Node-V1 Iroh daemon. NO installer or root authority.
//! Only a separately owner-provisioned systemd unit may start this listener.
mod identity;

use aquilum_core::node_v1::NodeHost;
use iroh::{RelayMode, endpoint::{Endpoint, presets}};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::{sync::{Mutex, Semaphore}, task::JoinSet};

const STATE: &str = "/var/lib/akula-node";
const ALPN: &[u8] = b"akula-node-v1/quic/v1";
const UDP_ADDRESS: &str = "0.0.0.0:43871";
const MAX_CLIENTS: usize = 8;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(40);

pub async fn run_provisioned_iroh() -> Result<(), &'static str> {
    if rustix::process::geteuid().as_raw() == 0 {
        return Err("Node daemon refuses root");
    }
    // Root policy must be present BEFORE any public UDP socket opens.
    let host = NodeHost::open_provisioned().map_err(|_| "Node owner policy HOLD")?;
    let secret = identity::load_provisioned(Path::new(STATE))?;
    host.verify_iroh_endpoint(*secret.public().as_bytes())
        .map_err(|_| "Iroh EndpointId is not root-pinned")?;
    let endpoint = Endpoint::builder(presets::Minimal)
        .secret_key(secret)
        .relay_mode(RelayMode::Disabled)
        .clear_ip_transports()
        .bind_addr(UDP_ADDRESS).map_err(|_| "Node fixed port invalid")?
        .alpns(vec![ALPN.to_vec()])
        .bind().await.map_err(|_| "Node UDP listener unavailable")?;
    let endpoint_id = endpoint.id();
    // These are public identifiers, not signing keys or owner-grant data.
    println!("AKULA_NODE_V1_LISTENING {endpoint_id} udp/43871");

    let host = Arc::new(Mutex::new(host));
    let quota = Arc::new(Semaphore::new(MAX_CLIENTS));
    let mut sessions = JoinSet::new();
    let mut term = tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    ).map_err(|_| "SIGTERM handler unavailable")?;

    loop {
        tokio::select! {
            _ = term.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break; };
                // Reject overload before TLS handshake or reading a message.
                let Ok(permit) = quota.clone().try_acquire_owned() else {
                    continue;
                };
                let host = Arc::clone(&host);
                sessions.spawn(async move {
                    let _permit = permit;
                    let handshake = tokio::time::timeout(
                        HANDSHAKE_TIMEOUT,
                        async {
                            let connecting = incoming.accept().map_err(|_| ())?;
                            connecting.await.map_err(|_| ())
                        },
                    ).await;
                    let Ok(Ok(conn)) = handshake else { return; };
                    // The shared SQLite ledger is not allowed to race between
                    // sessions. Bound every caller, including unauthenticated
                    // ones, to prevent a permanent stuck authority lock.
                    let mut locked = host.lock().await;
                    let _ = tokio::time::timeout(
                        OPERATION_TIMEOUT,
                        super::serve_one_authenticated_connection(&conn, &mut locked),
                    ).await;
                });
            },
            done = sessions.join_next(), if !sessions.is_empty() => {
                // Individual connections are untrusted: no per-peer errors,
                // paths, file names or signed grants are printed to logs.
                let _ = done;
            },
        }
    }
    sessions.abort_all();
    while sessions.join_next().await.is_some() {}
    endpoint.close().await;
    Ok(())
}
