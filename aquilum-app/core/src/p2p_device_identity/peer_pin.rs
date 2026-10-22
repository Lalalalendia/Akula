//! Durable, user-confirmed Iroh peer pins. No file, Vault, or Staya grants.
//! Transport MUST supply independently authenticated remote EndpointId bytes;
//! passing a value from an untrusted peer's message is not authentication.

#[cfg(windows)]
use super::windows_store;
mod contacts;
use super::{load, IdentityError};
pub use contacts::{list_peers, TransportPeerContact};
use zeroize::Zeroizing;

const PEER_RECORD_MAGIC: &[u8; 8] = b"AKPPIN01";
const PEER_RECORD_BYTES: usize = 73;
#[cfg(windows)]
const PEER_TARGET_PREFIX: &str = "Lalalalendia/Akula/P2P/TrustedPeer/v1/";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerState {
    NotFound,
    Confirmed,
    Revoked,
}

fn hex_to_endpoint(text: &str) -> Result<[u8; 32], IdentityError> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(IdentityError::InvalidPeerId);
    }
    let mut public = [0u8; 32];
    for (i, byte) in public.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16)
            .map_err(|_| IdentityError::InvalidPeerId)?;
    }
    if public == [0; 32] {
        return Err(IdentityError::InvalidPeerId);
    }
    Ok(public)
}

fn endpoint_hex(public: &[u8; 32]) -> String {
    public.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(windows)]
fn peer_target(remote: &[u8; 32]) -> String {
    format!("{PEER_TARGET_PREFIX}{}", endpoint_hex(remote))
}

fn encode_record(
    local: &[u8; 32],
    remote: &[u8; 32],
    state: PeerState,
) -> Zeroizing<[u8; PEER_RECORD_BYTES]> {
    let mut record = Zeroizing::new([0u8; PEER_RECORD_BYTES]);
    record[..8].copy_from_slice(PEER_RECORD_MAGIC);
    record[8..40].copy_from_slice(local);
    record[40..72].copy_from_slice(remote);
    record[72] = match state {
        PeerState::Confirmed => 1,
        PeerState::Revoked => 2,
        PeerState::NotFound => 0,
    };
    record
}

fn decode_record(
    record: &[u8],
    expected_local: &[u8; 32],
    expected_remote: &[u8; 32],
) -> Result<PeerState, IdentityError> {
    if record.len() != PEER_RECORD_BYTES
        || &record[..8] != PEER_RECORD_MAGIC
        || record[8..40] != *expected_local
        || record[40..72] != *expected_remote
    {
        return Err(IdentityError::CorruptCredential);
    }
    match record[72] {
        1 => Ok(PeerState::Confirmed),
        2 => Ok(PeerState::Revoked),
        _ => Err(IdentityError::CorruptCredential),
    }
}

#[cfg(windows)]
fn status_named(
    target: &str,
    local: &[u8; 32],
    remote: &[u8; 32],
) -> Result<PeerState, IdentityError> {
    match windows_store::read_fixed::<PEER_RECORD_BYTES>(target) {
        Ok(record) => decode_record(&record[..], local, remote),
        Err(IdentityError::NotInitialized) => Ok(PeerState::NotFound),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn confirm_named(target: &str, local: &[u8; 32], remote: &[u8; 32]) -> Result<(), IdentityError> {
    let _lock = windows_store::EnrollmentLock::acquire()?;
    match status_named(target, local, remote)? {
        PeerState::NotFound => {}
        PeerState::Confirmed => return Err(IdentityError::AlreadyInitialized),
        PeerState::Revoked => return Err(IdentityError::PeerRevoked),
    }
    let mut record = encode_record(local, remote, PeerState::Confirmed);
    windows_store::write_fixed(target, &mut record)?;
    if status_named(target, local, remote)? != PeerState::Confirmed {
        return Err(IdentityError::EnrollmentChanged);
    }
    Ok(())
}

#[cfg(windows)]
fn revoke_named(target: &str, local: &[u8; 32], remote: &[u8; 32]) -> Result<(), IdentityError> {
    let _lock = windows_store::EnrollmentLock::acquire()?;
    match status_named(target, local, remote)? {
        PeerState::NotFound => return Err(IdentityError::PeerNotFound),
        PeerState::Revoked => return Ok(()),
        PeerState::Confirmed => {}
    }
    let mut tombstone = encode_record(local, remote, PeerState::Revoked);
    windows_store::write_fixed(target, &mut tombstone)?;
    if status_named(target, local, remote)? != PeerState::Revoked {
        return Err(IdentityError::EnrollmentChanged);
    }
    Ok(())
}

/// Requires two complete, independently compared 64-digit lowercase EndpointIds.
/// UI must collect the second value from a truly independent human confirmation,
/// not auto-fill it from the discovered peer or from the same network message.
/// A revoked pin cannot silently re-enroll.
pub fn confirm_peer(discovered_id: &str, human_verified_id: &str) -> Result<(), IdentityError> {
    let remote = hex_to_endpoint(discovered_id)?;
    let confirmed = hex_to_endpoint(human_verified_id)?;
    if remote != confirmed {
        return Err(IdentityError::PeerMismatch);
    }
    let local = load()?.endpoint_public_bytes();
    if remote == local {
        return Err(IdentityError::InvalidPeerId);
    }
    #[cfg(windows)]
    {
        confirm_named(&peer_target(&remote), &local, &remote)
    }
    #[cfg(not(windows))]
    {
        let _ = (local, remote);
        Err(IdentityError::UnsupportedPlatform)
    }
}

/// Read fresh OS-persisted status. No cached "allowed peers" snapshot.
pub fn peer_state(remote_id: &str) -> Result<PeerState, IdentityError> {
    let remote = hex_to_endpoint(remote_id)?;
    let local = load()?.endpoint_public_bytes();
    #[cfg(windows)]
    {
        status_named(&peer_target(&remote), &local, &remote)
    }
    #[cfg(not(windows))]
    {
        let _ = (local, remote);
        Err(IdentityError::UnsupportedPlatform)
    }
}

/// Revoke for future operations. Records a persistent deny tombstone; does not
/// delete it and never kills already in-flight QUIC bytes retroactively.
pub fn revoke_peer(remote_id: &str) -> Result<(), IdentityError> {
    let remote = hex_to_endpoint(remote_id)?;
    let local = load()?.endpoint_public_bytes();
    #[cfg(windows)]
    {
        revoke_named(&peer_target(&remote), &local, &remote)
    }
    #[cfg(not(windows))]
    {
        let _ = (local, remote);
        Err(IdentityError::UnsupportedPlatform)
    }
}

/// Call before EVERY new logical operation, supplying the actual verified QUIC
/// remote_id() and the user's intended selected contact. This is transport
/// admission only; independent sender consent and File Gate are still needed.
pub fn require_pinned_remote(
    selected_full_id: &str,
    authenticated_remote: &[u8; 32],
) -> Result<(), IdentityError> {
    let expected = hex_to_endpoint(selected_full_id)?;
    if expected != *authenticated_remote {
        return Err(IdentityError::PeerMismatch);
    }
    match peer_state(selected_full_id)? {
        PeerState::Confirmed => Ok(()),
        PeerState::Revoked => Err(IdentityError::PeerRevoked),
        PeerState::NotFound => Err(IdentityError::PeerNotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_exact_full_lowercase_ids() {
        let valid = endpoint_hex(&[0x23; 32]);
        assert_eq!(hex_to_endpoint(&valid).unwrap(), [0x23; 32]);
        for bad in [
            "".to_string(),
            "abc".to_string(),
            "A".repeat(64),
            "0".repeat(64),
            "g".repeat(64),
        ] {
            assert!(hex_to_endpoint(&bad).is_err());
        }
        let local = [9; 32];
        let remote = [8; 32];
        let record = encode_record(&local, &remote, PeerState::Confirmed);
        assert_eq!(
            decode_record(&record[..], &local, &remote),
            Ok(PeerState::Confirmed)
        );
        assert_eq!(
            decode_record(&record[..], &remote, &local),
            Err(IdentityError::CorruptCredential)
        );
        let revoked = encode_record(&local, &remote, PeerState::Revoked);
        assert_eq!(
            decode_record(&revoked[..], &local, &remote),
            Ok(PeerState::Revoked)
        );
        let mut corrupted = revoked.clone();
        corrupted[72] = 0;
        assert!(matches!(
            decode_record(&corrupted[..], &local, &remote),
            Err(IdentityError::CorruptCredential)
        ));
    }

    #[test]
    fn false_manual_confirmation_has_no_store_side_effects() {
        let a = endpoint_hex(&[13; 32]);
        let b = endpoint_hex(&[14; 32]);
        assert!(matches!(
            confirm_peer(&a, &b),
            Err(IdentityError::PeerMismatch)
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_backend_cannot_enroll_or_revoke() {
        let id = endpoint_hex(&[18; 32]);
        assert!(matches!(
            confirm_peer(&id, &id),
            Err(IdentityError::UnsupportedPlatform)
        ));
        assert!(matches!(
            revoke_peer(&id),
            Err(IdentityError::UnsupportedPlatform)
        ));
        assert!(matches!(
            peer_state(&id),
            Err(IdentityError::UnsupportedPlatform)
        ));
        assert!(require_pinned_remote(&id, &[18; 32]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_peer_pin_persists_across_real_process_and_revocation() {
        struct Cleanup(String, String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                windows_store::delete_named_test_only(&self.0);
                windows_store::delete_named_test_only(&self.1);
            }
        }
        let scope = uuid::Uuid::new_v4().to_string();
        let local_target = format!("Lalalalendia/Akula/P2P/test/{scope}/local");
        let peer_target = format!("Lalalalendia/Akula/P2P/test/{scope}/peer");
        let _cleanup = Cleanup(local_target.clone(), peer_target.clone());
        let local = windows_store::initialize_named(&local_target)
            .unwrap()
            .endpoint_public_bytes();
        let remote = [21; 32];
        assert_eq!(
            status_named(&peer_target, &local, &remote),
            Ok(PeerState::NotFound)
        );
        confirm_named(&peer_target, &local, &remote).unwrap();
        assert!(matches!(
            confirm_named(&peer_target, &local, &remote),
            Err(IdentityError::AlreadyInitialized)
        ));
        let check_child = |state: &str| {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg("p2p_device_identity::peer_pin::tests::windows_child_reads_pinned_peer")
                .arg("--nocapture")
                .env("AKULA_P2P_TEST_LOCAL_TARGET", &local_target)
                .env("AKULA_P2P_TEST_PEER_TARGET", &peer_target)
                .env("AKULA_P2P_TEST_PEER_STATE", state)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "second process could not read persisted peer state"
            );
        };
        check_child("confirmed");
        revoke_named(&peer_target, &local, &remote).unwrap();
        assert_eq!(
            status_named(&peer_target, &local, &remote),
            Ok(PeerState::Revoked)
        );
        assert!(matches!(
            confirm_named(&peer_target, &local, &remote),
            Err(IdentityError::PeerRevoked)
        ));
        check_child("revoked");
    }

    #[cfg(windows)]
    #[test]
    fn windows_child_reads_pinned_peer() {
        let Ok(local_target) = std::env::var("AKULA_P2P_TEST_LOCAL_TARGET") else {
            return;
        };
        let peer_target = std::env::var("AKULA_P2P_TEST_PEER_TARGET").unwrap();
        let expected = std::env::var("AKULA_P2P_TEST_PEER_STATE").unwrap();
        let local = windows_store::load_named(&local_target)
            .unwrap()
            .endpoint_public_bytes();
        let remote = [21; 32];
        let state = status_named(&peer_target, &local, &remote).unwrap();
        assert_eq!(
            state,
            if expected == "confirmed" {
                PeerState::Confirmed
            } else {
                PeerState::Revoked
            }
        );
    }
}
