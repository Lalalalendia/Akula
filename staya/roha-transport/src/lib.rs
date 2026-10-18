//! Optional Akula "Staya" transport built on the tested Roha -> noq fork chain.
//!
//! This crate is deliberately isolated from Akula's Sync Core and File Gate.
//! An authenticated QUIC peer is not automatically authorized to see files.

use std::collections::BTreeSet;
use std::time::Duration;

use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, endpoint::presets};

/// Network protocol reserved for a Staya connection, not a file-write capability.
pub const STAYA_ALPN: &[u8] = b"akula/staya/transport/0";

pub mod admission;
pub mod transfer;

/// Explicitly authorized endpoint keys. The pairing workflow owns this list.
#[derive(Debug, Default, Clone)]
pub struct AllowedPeers(BTreeSet<EndpointId>);

impl AllowedPeers {
    pub fn new(peers: impl IntoIterator<Item = EndpointId>) -> Self {
        Self(peers.into_iter().collect())
    }

    pub fn contains(&self, id: &EndpointId) -> bool {
        self.0.contains(id)
    }
}

/// A network endpoint without implicit n0 relay or DNS publication.
#[derive(Debug)]
pub struct StayaTransport {
    endpoint: Endpoint,
    peers: AllowedPeers,
}

impl StayaTransport {
    /// Bind a local endpoint; the caller owns key persistence and peer enrollment.
    /// Does not start a background sync worker or use n0 public services.
    pub async fn bind(secret: SecretKey, peers: AllowedPeers) -> Result<Self, String> {
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![STAYA_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .map_err(|err| format!("bind: {err}"))?;
        Ok(Self { endpoint, peers })
    }

    pub fn address(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Open a connection only when its expected key has already been admitted.
    pub async fn connect(&self, addr: EndpointAddr) -> Result<iroh::endpoint::Connection, String> {
        if !self.peers.contains(&addr.id) {
            return Err("peer not admitted".into());
        }
        tokio::time::timeout(
            Duration::from_secs(15),
            self.endpoint.connect(addr, STAYA_ALPN),
        )
        .await
        .map_err(|_| "connect timeout".to_owned())?
        .map_err(|err| format!("connect: {err}"))
    }

    /// Wait for one authorized incoming connection. Unknown keys are closed.
    pub async fn accept_one(&self) -> Result<iroh::endpoint::Connection, String> {
        loop {
            let incoming = self.endpoint.accept().await.ok_or("endpoint stopped")?;
            let connecting = incoming.accept().map_err(|err| format!("accept: {err}"))?;
            let conn = match tokio::time::timeout(Duration::from_secs(15), connecting).await {
                Ok(Ok(conn)) => conn,
                Ok(Err(err)) => return Err(format!("handshake: {err}")),
                Err(_) => return Err("handshake timeout".into()),
            };
            if self.peers.contains(&conn.remote_id()) {
                return Ok(conn);
            }
            conn.close(1u8.into(), b"unknown peer");
        }
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_peers_are_denied() {
        let permitted = SecretKey::generate().public();
        let other = SecretKey::generate().public();
        let policy = AllowedPeers::new([permitted]);
        assert!(policy.contains(&permitted));
        assert!(!policy.contains(&other));
        assert!(!AllowedPeers::default().contains(&permitted));
    }
}
