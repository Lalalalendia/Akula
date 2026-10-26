//! GNET-03: experimental QUIC-based LAN transport using pinned Roha.
//!
//! Explicitly no external relay, public DNS lookup, automatic router port
//! mapping or automatic captive-portal checks. Bind/connect only to loopback,
//! RFC1918 IPv4 or IPv6 ULA/link-local for the first, LAN-only experiment.
//! The absence of WAN traffic MUST still be verified with a packet capture.
//!
//! The synchronous GameTransport contract is bridged through two small Tokio
//! tasks. ReceivedFrame.from comes from the authenticated QUIC connection,
//! never from the untrusted serialized command. No 0-RTT is used.

use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use akula_game_session::{
    PeerId,
    transport::{GameTransport, MAX_FRAME_BYTES, MAX_QUEUED_FRAMES, ReceivedFrame, TransportError},
};
use anyhow::{Context, Result, anyhow, ensure};
use iroh::{
    Endpoint, EndpointAddr, EndpointId,
    endpoint::{Connection, NetReportConfig, PortmapperConfig, presets},
};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{sleep, timeout},
};

pub const GAME_ALPN: &[u8] = b"akula-game-lan/1";
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(12);

/// Conservative v1-only address policy. Routed public ranges are intentionally
/// unavailable in strict LAN-only mode. This does not prove network isolation.
pub fn is_lan_address(addr: SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback(),
    }
}

/// No listener exists until the explicit caller invokes this function.
pub async fn bind_lan(addr: SocketAddr) -> Result<Endpoint> {
    ensure!(
        is_lan_address(addr),
        "LAN-only mode rejects nonlocal bind address"
    );
    let endpoint = Endpoint::builder(presets::Minimal)
        .alpns(vec![GAME_ALPN.to_vec()])
        .clear_address_lookup()
        .clear_relay_transports()
        .portmapper_config(PortmapperConfig::Disabled)
        .net_report_config(NetReportConfig::minimal())
        .clear_ip_transports()
        .bind_addr(addr)
        .map_err(|e| anyhow!("LAN socket configuration: {e:?}"))?
        .bind()
        .await
        .map_err(|e| anyhow!("LAN bind: {e:?}"))?;
    ensure!(
        endpoint.addr().relay_urls().next().is_none(),
        "LAN-only mode unexpectedly advertised a relay"
    );
    Ok(endpoint)
}

/// The host accepts one full authenticated QUIC handshake.
/// User approval of this authenticated peer is a separate caller action.
pub async fn accept_lan(endpoint: &Endpoint) -> Result<Connection> {
    let incoming = timeout(HANDSHAKE_TIMEOUT * 4, endpoint.accept())
        .await
        .context("no LAN connection within timeout")?
        .context("LAN endpoint was closed")?;
    let conn = timeout(HANDSHAKE_TIMEOUT, incoming)
        .await
        .context("incoming QUIC handshake timeout")?
        .map_err(|e| anyhow!("incoming QUIC handshake failed: {e:?}"))?;
    Ok(conn)
}

/// Guest connects directly to the known host key plus literal LAN socket.
/// DNS address lookup and relays are intentionally not available.
pub async fn connect_lan(
    endpoint: &Endpoint,
    expected_host: EndpointId,
    host_addr: SocketAddr,
) -> Result<Connection> {
    ensure!(
        is_lan_address(host_addr),
        "WAN address refused in LAN-only mode"
    );
    ensure!(expected_host != endpoint.id(), "refusing self-connect");
    let direct = EndpointAddr::new(expected_host).with_ip_addr(host_addr);
    let conn = timeout(HANDSHAKE_TIMEOUT, endpoint.connect(direct, GAME_ALPN))
        .await
        .context("QUIC connection timed out")?
        .map_err(|e| anyhow!("LAN connect: {e:?}"))?;
    ensure!(
        conn.remote_id() == expected_host,
        "remote authenticated endpoint differs from expected host"
    );
    Ok(conn)
}

/// A single already authenticated and approved peer.
pub struct RohaLanTransport {
    local: PeerId,
    remote: PeerId,
    outbound: mpsc::Sender<Vec<u8>>,
    inbound: mpsc::Receiver<ReceivedFrame>,
    writer: JoinHandle<()>,
    reader: JoinHandle<()>,
}

impl RohaLanTransport {
    pub fn new(endpoint: &Endpoint, conn: Connection) -> Self {
        let local = PeerId(*endpoint.id().as_bytes());
        let remote = PeerId(*conn.remote_id().as_bytes());
        let (outbound, mut outgoing_frames) = mpsc::channel::<Vec<u8>>(MAX_QUEUED_FRAMES);
        let (incoming_frames, inbound) = mpsc::channel::<ReceivedFrame>(MAX_QUEUED_FRAMES);

        let send_connection = conn.clone();
        let writer = tokio::spawn(async move {
            while let Some(data) = outgoing_frames.recv().await {
                let Ok(mut stream) = send_connection.open_uni().await else {
                    break;
                };
                if stream.write_all(&data).await.is_err() {
                    break;
                }
                if stream.finish().is_err() {
                    break;
                }
            }
        });
        let reader = tokio::spawn(async move {
            while let Ok(mut stream) = conn.accept_uni().await {
                let Ok(data) = stream.read_to_end(MAX_FRAME_BYTES).await else {
                    break;
                };
                if data.is_empty() {
                    break;
                }
                if incoming_frames
                    .send(ReceivedFrame {
                        from: remote,
                        payload: data,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            local,
            remote,
            outbound,
            inbound,
            writer,
            reader,
        }
    }

    pub fn remote_peer(&self) -> PeerId {
        self.remote
    }
}

impl GameTransport for RohaLanTransport {
    fn local_peer(&self) -> PeerId {
        self.local
    }

    fn send(&mut self, destination: PeerId, bytes: &[u8]) -> Result<(), TransportError> {
        if destination != self.remote {
            return Err(TransportError::WrongDestination);
        }
        if bytes.is_empty() {
            return Err(TransportError::EmptyFrame);
        }
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(TransportError::OversizedFrame);
        }
        self.outbound.try_send(bytes.to_vec()).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => TransportError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
        })
    }

    fn try_recv(&mut self) -> Result<Option<ReceivedFrame>, TransportError> {
        match self.inbound.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(TransportError::Closed),
        }
    }
}

impl Drop for RohaLanTransport {
    fn drop(&mut self) {
        self.writer.abort();
        self.reader.abort();
    }
}

/// Wait for the next verified-frame source, with a bounded timeout.
/// A network adapter closing is not an empty packet and must fail.
pub async fn next_frame(
    transport: &mut impl GameTransport,
    limit: Duration,
) -> Result<ReceivedFrame> {
    timeout(limit, async {
        loop {
            match transport.try_recv() {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => sleep(Duration::from_millis(5)).await,
                Err(err) => return Err(anyhow!("transport receive failed: {err:?}")),
            }
        }
    })
    .await
    .context("no game frame within deadline")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_address_policy_rejects_public_and_unspecified() {
        for good in [
            "127.0.0.1:1010",
            "192.168.1.2:10",
            "10.10.1.2:10",
            "172.19.0.1:10",
            "[::1]:10",
            "[fd01::2]:10",
        ] {
            assert!(is_lan_address(good.parse().unwrap()), "{good}");
        }
        for bad in [
            "0.0.0.0:10",
            "8.8.8.8:53",
            "1.1.1.1:53",
            "[::]:10",
            "[2606:4700:4700::1111]:53",
        ] {
            assert!(!is_lan_address(bad.parse().unwrap()), "{bad}");
        }
    }

    #[tokio::test]
    async fn loopback_authenticated_frames_work_without_relay() {
        let host = bind_lan("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let guest = bind_lan("127.0.0.1:0".parse().unwrap()).await.unwrap();
        assert_eq!(host.addr().relay_urls().count(), 0);
        assert_eq!(guest.addr().relay_urls().count(), 0);
        let host_addr = host.bound_sockets()[0];
        let host_id = host.id();
        let (accepted, connected) =
            tokio::join!(accept_lan(&host), connect_lan(&guest, host_id, host_addr));
        let host_conn = accepted.unwrap();
        let guest_conn = connected.unwrap();
        assert_eq!(host_conn.remote_id(), guest.id());
        assert_eq!(guest_conn.remote_id(), host.id());
        let mut host_t = RohaLanTransport::new(&host, host_conn);
        let mut guest_t = RohaLanTransport::new(&guest, guest_conn);

        guest_t.send(host_t.local_peer(), b"first-move").unwrap();
        let received = next_frame(&mut host_t, FRAME_TIMEOUT).await.unwrap();
        assert_eq!(received.from, guest_t.local_peer());
        assert_eq!(received.payload, b"first-move");
        host_t.send(guest_t.local_peer(), b"ack").unwrap();
        let reply = next_frame(&mut guest_t, FRAME_TIMEOUT).await.unwrap();
        assert_eq!(reply.from, host_t.local_peer());
        assert_eq!(reply.payload, b"ack");

        drop(host_t);
        drop(guest_t);
        host.close().await;
        guest.close().await;
    }
}
