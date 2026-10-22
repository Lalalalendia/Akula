//! Optional Akula OS-pinned Iroh admission for the existing Staya bridge.
//! Transport identity only: no Vault grant, file access or durable receipt.
//! Missing, revoked or malformed OS credentials always deny a new operation.

use aquilum_core::p2p_device_identity::{self, peer_pin};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, SecretKey, endpoint::presets};
use std::{net::SocketAddr, str::FromStr, time::Duration};

pub const OS_PIN_ALPN: &[u8] = b"akula/staya/os-pin/1";

fn protected_secret() -> Result<(SecretKey, [u8; 32]), String> {
    let identity = p2p_device_identity::load().map_err(|error| error.to_string())?;
    let public = identity.endpoint_public_bytes();
    // This never creates a new key or exposes the OS-held seed to a log/file.
    let secret = identity.with_secret_seed(SecretKey::from_bytes);
    Ok((secret, public))
}

fn check_selected(selected: &str) -> Result<EndpointId, String> {
    let expected = EndpointId::from_str(selected).map_err(|_| "invalid selected peer ID")?;
    peer_pin::require_pinned_remote(selected, expected.as_bytes())
        .map_err(|error| format!("OS peer pin: {error}"))?;
    Ok(expected)
}

pub struct OsPinnedEndpoint {
    endpoint: Endpoint,
}

impl OsPinnedEndpoint {
    /// Direct-only endpoint using the persistent Windows Credential Manager key.
    pub async fn bind() -> Result<Self, String> {
        let (secret, public) = protected_secret()?;
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![OS_PIN_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .map_err(|error| format!("QUIC bind: {error}"))?;
        if endpoint.id().as_bytes() != &public {
            endpoint.close().await;
            return Err("OS key differs from QUIC EndpointId".into());
        }
        Ok(Self { endpoint })
    }

    /// Same protected identity, constrained to loopback for CI only.
    #[cfg(feature = "os-pinned-lab")]
    pub async fn bind_loopback_lab() -> Result<Self, String> {
        let (secret, public) = protected_secret()?;
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(secret)
            .alpns(vec![OS_PIN_ALPN.to_vec()])
            .relay_mode(RelayMode::Disabled)
            .clear_ip_transports()
            .bind_addr("127.0.0.1:0")
            .map_err(|error| format!("loopback address: {error}"))?
            .bind()
            .await
            .map_err(|error| format!("loopback bind: {error}"))?;
        if endpoint.id().as_bytes() != &public {
            endpoint.close().await;
            return Err("OS key differs from QUIC EndpointId".into());
        }
        Ok(Self { endpoint })
    }

    pub fn id(&self) -> EndpointId {
        self.endpoint.id()
    }

    pub fn bound_sockets(&self) -> Vec<SocketAddr> {
        self.endpoint.bound_sockets().into_iter().collect()
    }

    #[cfg(feature = "os-pinned-lab")]
    pub fn loopback_address_lab(&self) -> Result<EndpointAddr, String> {
        let socket = self
            .bound_sockets()
            .into_iter()
            .find(|socket| socket.ip().is_loopback())
            .ok_or_else(|| "no loopback socket".to_owned())?;
        Ok(EndpointAddr::new(self.id()).with_ip_addr(socket))
    }

    /// Check the selected OS peer against both advertised and live TLS ID.
    pub async fn connect_selected(
        &self,
        selected: &str,
        address: EndpointAddr,
    ) -> Result<VerifiedPeerConnection, String> {
        let expected = check_selected(selected)?;
        if address.id != expected {
            return Err("selected peer differs from advertised address".into());
        }
        let connection = tokio::time::timeout(
            Duration::from_secs(15),
            self.endpoint.connect(address, OS_PIN_ALPN),
        )
        .await
        .map_err(|_| "QUIC connection timeout".to_owned())?
        .map_err(|error| format!("QUIC connection: {error}"))?;
        VerifiedPeerConnection::new(selected, connection)
    }

    /// Admit one selected, OS-confirmed key; never trust the incoming claim.
    pub async fn accept_selected(
        &self,
        selected: &str,
    ) -> Result<VerifiedPeerConnection, String> {
        check_selected(selected)?;
        let incoming = tokio::time::timeout(Duration::from_secs(60), self.endpoint.accept())
            .await
            .map_err(|_| "waiting for QUIC peer timed out".to_owned())?
            .ok_or_else(|| "QUIC endpoint stopped".to_owned())?;
        let connecting = incoming
            .accept()
            .map_err(|error| format!("incoming QUIC: {error}"))?;
        let connection = tokio::time::timeout(Duration::from_secs(15), connecting)
            .await
            .map_err(|_| "QUIC handshake timed out".to_owned())?
            .map_err(|error| format!("QUIC handshake: {error}"))?;
        VerifiedPeerConnection::new(selected, connection)
    }

    pub async fn close(&self) {
        self.endpoint.close().await;
    }
}

/// Only transport admission. File Gate must authorize the requested object
/// separately. Never cache an authorization across new logical operations.
pub struct VerifiedPeerConnection {
    connection: iroh::endpoint::Connection,
    selected: String,
}

impl VerifiedPeerConnection {
    fn new(selected: &str, connection: iroh::endpoint::Connection) -> Result<Self, String> {
        if let Err(error) =
            peer_pin::require_pinned_remote(selected, connection.remote_id().as_bytes())
        {
            connection.close(403u32.into(), b"peer pin missing or revoked");
            return Err(format!("authenticated peer denied: {error}"));
        }
        Ok(Self {
            connection,
            selected: selected.to_owned(),
        })
    }

    pub fn remote_id(&self) -> EndpointId {
        self.connection.remote_id()
    }

    /// Re-read the persistent revoke tombstone before EVERY new operation.
    pub fn authorize_new_operation(&self) -> Result<&iroh::endpoint::Connection, String> {
        peer_pin::require_pinned_remote(&self.selected, self.connection.remote_id().as_bytes())
            .map_err(|error| format!("new operation denied: {error}"))?;
        Ok(&self.connection)
    }
}
