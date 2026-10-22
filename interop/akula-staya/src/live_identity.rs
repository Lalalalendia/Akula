//! Live Iroh QUIC identity + fresh Core OS peer pin. No file grants.
use aquilum_core::p2p_device_identity::{self, IdentityError, peer_pin};
use iroh::{SecretKey, endpoint::Connection};

/// Use the real OS-protected Akula seed. Never accept a key from a ticket,
/// plaintext settings, a file path, or a UI field.
pub fn with_protected_iroh_secret<T>(
    use_key: impl FnOnce(SecretKey) -> T,
) -> Result<T, IdentityError> {
    let identity = p2p_device_identity::load()?;
    let expected_public = identity.endpoint_public_bytes();
    identity.with_secret_seed(|seed| {
        let key = SecretKey::from_bytes(seed);
        if key.public().as_bytes() != &expected_public {
            return Err(IdentityError::CorruptCredential);
        }
        Ok(use_key(key))
    })
}

/// Call before every new logical operation with the actual authenticated
/// Connection and the independently user-selected full 64-digit endpoint ID.
/// Passing this gate never grants Staya GET, Vault or File Gate authority.
pub fn require_pinned_live_peer(
    connection: &Connection,
    selected_full_endpoint_id: &str,
) -> Result<(), IdentityError> {
    peer_pin::require_pinned_remote(selected_full_endpoint_id, connection.remote_id().as_bytes())
}
