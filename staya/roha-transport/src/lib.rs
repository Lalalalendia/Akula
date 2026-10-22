//! Optional Akula "Staya" transport built on the tested Roha -> noq fork chain.
//!
//! This crate is deliberately isolated from Akula's Sync Core and File Gate.
//! An authenticated QUIC peer is not automatically authorized to see files.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, endpoint::presets};

/// Network protocol reserved for a Staya connection, not a file-write capability.
pub const STAYA_ALPN: &[u8] = b"akula/staya/transport/0";

pub mod transfer;

/// Network admission only: neither a Vault grant nor production pairing.
/// All clones share revocations; poisoned locks fail closed. A live verifier
/// may re-read Akula's protected peer pins for each authenticated operation.
#[derive(Clone, Default)]
pub struct AllowedPeers {
    peers: Arc<RwLock<BTreeSet<EndpointId>>>,
    live_check: Option<Arc<dyn Fn(&EndpointId) -> bool + Send + Sync>>,
}

impl std::fmt::Debug for AllowedPeers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllowedPeers")
            .field("has_live_check", &self.live_check.is_some())
            .finish_non_exhaustive()
    }
}

impl AllowedPeers {
    pub fn new(peers: impl IntoIterator<Item = EndpointId>) -> Self {
        Self {
            peers: Arc::new(RwLock::new(peers.into_iter().collect())),
            live_check: None,
        }
    }

    /// Install a fail-closed, freshly evaluated *host-owned* peer admission
    /// check. Akula must supply a closure that calls
    /// p2p_device_identity::peer_pin::require_pinned_remote with this REAL
    /// connection's remote_id(). Never trust a key copied from a wire payload,
    /// contact row or invitation. This is transport admission, not a file grant.
    pub fn with_live_check(
        mut self,
        check: impl Fn(&EndpointId) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.live_check = Some(Arc::new(check));
        self
    }

    pub fn contains(&self, id: &EndpointId) -> bool {
        if !self.peers.read().is_ok_and(|peers| peers.contains(id)) {
            return false;
        }
        self.live_check.as_ref().is_none_or(|check| check(id))
    }

    /// Revokes admission in this process. No durable or distributed policy.
    pub fn revoke(&self, id: &EndpointId) -> bool {
        self.peers.write().is_ok_and(|mut peers| peers.remove(id))
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
        let expected = addr.id;
        let conn = tokio::time::timeout(
            Duration::from_secs(15),
            self.endpoint.connect(addr, STAYA_ALPN),
        )
        .await
        .map_err(|_| "connect timeout".to_owned())?
        .map_err(|err| format!("connect: {err}"))?;
        if conn.remote_id() != expected || !self.peers.contains(&conn.remote_id()) {
            conn.close(403u32.into(), b"peer revoked during handshake");
            return Err("peer revoked during handshake".into());
        }
        Ok(conn)
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

    /// A fresh network-admission check for each NEW logical operation on
    /// an already-authenticated QUIC session. It does NOT grant file access.
    pub fn authorize_new_operation(
        &self,
        connection: &iroh::endpoint::Connection,
    ) -> Result<(), String> {
        if self.peers.contains(&connection.remote_id()) {
            Ok(())
        } else {
            Err("peer not admitted for new operation".into())
        }
    }

    #[cfg(feature = "process-lab")]
    pub async fn bind_loopback_lab(secret: SecretKey, peers: AllowedPeers) -> Result<Self, String> {
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![STAYA_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .map_err(|err| format!("loopback address: {err}"))?
            .bind()
            .await
            .map_err(|err| format!("loopback bind: {err}"))?;
        Ok(Self { endpoint, peers })
    }

    #[cfg(feature = "process-lab")]
    pub fn loopback_address_lab(&self) -> Result<EndpointAddr, String> {
        let addr = self
            .endpoint
            .bound_sockets()
            .into_iter()
            .find(|socket| socket.is_ipv4() && socket.ip().is_loopback())
            .ok_or_else(|| "no loopback UDP address".to_owned())?;
        Ok(EndpointAddr::new(self.id()).with_ip_addr(addr))
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
        let visible = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let live = AllowedPeers::new([permitted]).with_live_check({
            let visible = Arc::clone(&visible);
            move |key| key == &permitted && visible.load(std::sync::atomic::Ordering::Acquire)
        });
        assert!(live.contains(&permitted));
        visible.store(false, std::sync::atomic::Ordering::Release);
        assert!(!live.contains(&permitted), "no cached permission after revoke");
    }
}
