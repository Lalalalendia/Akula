//! Read-only OS-backed transport contact enumeration for UI presentation.
//! Every snapshot is revalidated and fails as a whole if any entry is corrupt.
//! These are public transport pins, NOT verified Vault memberships or grants.
use super::{IdentityError, PeerState};

/// A display-only credential observation. It must never authorize a transfer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransportPeerContact {
    pub endpoint_id_hex: String,
    pub state: PeerState,
}

const MAX_PEER_CONTACTS: usize = 32;

#[cfg(windows)]
fn list_named(prefix: &str, local: &[u8; 32]) -> Result<Vec<TransportPeerContact>, IdentityError> {
    use super::{hex_to_endpoint, status_named};

    let names = enumerate_names(prefix)?;
    let mut entries = Vec::with_capacity(names.len());
    for target in names {
        let hex = target
            .strip_prefix(prefix)
            .ok_or(IdentityError::CorruptCredential)?;
        let remote = hex_to_endpoint(hex).map_err(|_| IdentityError::CorruptCredential)?;
        if remote == *local {
            return Err(IdentityError::CorruptCredential);
        }
        let state = status_named(&target, local, &remote)?;
        if state == PeerState::NotFound {
            return Err(IdentityError::CorruptCredential);
        }
        entries.push(TransportPeerContact {
            endpoint_id_hex: hex.to_owned(),
            state,
        });
    }
    entries.sort_by(|a, b| a.endpoint_id_hex.cmp(&b.endpoint_id_hex));
    if entries
        .windows(2)
        .any(|pair| pair[0].endpoint_id_hex == pair[1].endpoint_id_hex)
    {
        return Err(IdentityError::CorruptCredential);
    }
    Ok(entries)
}

#[cfg(windows)]
fn enumerate_names(prefix: &str) -> Result<Vec<String>, IdentityError> {
    use std::ptr;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::ERROR_NOT_FOUND,
            Security::Credentials::{CredEnumerateW, CredFree, CREDENTIALW, CRED_TYPE_GENERIC},
        },
    };

    struct Enumeration(*mut *mut CREDENTIALW);
    impl Drop for Enumeration {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CredFree(self.0.cast()) };
            }
        }
    }

    let wide: Vec<u16> = format!("{prefix}*")
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut count = 0u32;
    let mut pointer: *mut *mut CREDENTIALW = ptr::null_mut();
    match unsafe { CredEnumerateW(PCWSTR(wide.as_ptr()), None, &mut count, &mut pointer) } {
        Ok(()) => {}
        Err(error) if error.code() == ERROR_NOT_FOUND.to_hresult() => return Ok(Vec::new()),
        Err(_) => return Err(IdentityError::StoreUnavailable),
    }
    let allocation = Enumeration(pointer);
    if count as usize > MAX_PEER_CONTACTS {
        return Err(IdentityError::CorruptCredential);
    }
    if count == 0 {
        return Ok(Vec::new());
    }
    if allocation.0.is_null() {
        return Err(IdentityError::StoreUnavailable);
    }
    let mut names = Vec::with_capacity(count as usize);
    let pointers = unsafe { std::slice::from_raw_parts(allocation.0, count as usize) };
    for &item in pointers {
        if item.is_null() {
            return Err(IdentityError::CorruptCredential);
        }
        let credential = unsafe { &*item };
        if credential.Type != CRED_TYPE_GENERIC || credential.TargetName.is_null() {
            return Err(IdentityError::CorruptCredential);
        }
        let name = unsafe { credential.TargetName.to_string() }
            .map_err(|_| IdentityError::CorruptCredential)?;
        if !name.starts_with(prefix) {
            return Err(IdentityError::CorruptCredential);
        }
        names.push(name);
    }
    Ok(names)
}

/// Lists the current user's OS-backed pins without granting them any network
/// or file permissions. A missing local key is an error, not an empty list.
pub fn list_peers() -> Result<Vec<TransportPeerContact>, IdentityError> {
    let local = super::load()?.endpoint_public_bytes();
    #[cfg(windows)]
    {
        list_named(super::PEER_TARGET_PREFIX, &local)
    }
    #[cfg(not(windows))]
    {
        let _ = local;
        Err(IdentityError::UnsupportedPlatform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn no_plaintext_or_mocked_contact_list_when_store_is_unsupported() {
        assert!(matches!(
            list_peers(),
            Err(IdentityError::UnsupportedPlatform)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn credential_enumeration_observes_revoke_and_refuses_malformed_entry() {
        use super::super::windows_store;
        use super::super::{confirm_named, encode_record, endpoint_hex, revoke_named};

        struct Cleanup(Vec<String>);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for target in &self.0 {
                    windows_store::delete_named_test_only(target);
                }
            }
        }

        let run = uuid::Uuid::new_v4().to_string();
        let prefix = format!("Lalalalendia/Akula/P2P/test/{run}/contact/");
        let owner_target = format!("Lalalalendia/Akula/P2P/test/{run}/owner");
        let remote = [19u8; 32];
        let remote_id = endpoint_hex(&remote);
        let contact_target = format!("{prefix}{remote_id}");
        let damaged_target = format!("{prefix}INVALID");
        let _cleanup = Cleanup(vec![
            owner_target.clone(),
            contact_target.clone(),
            damaged_target.clone(),
        ]);
        let local = windows_store::initialize_named(&owner_target)
            .unwrap()
            .endpoint_public_bytes();
        assert!(list_named(&prefix, &local).unwrap().is_empty());
        confirm_named(&contact_target, &local, &remote).unwrap();
        assert_eq!(
            list_named(&prefix, &local).unwrap(),
            vec![TransportPeerContact {
                endpoint_id_hex: remote_id,
                state: PeerState::Confirmed
            }]
        );
        revoke_named(&contact_target, &local, &remote).unwrap();
        assert_eq!(
            list_named(&prefix, &local).unwrap()[0].state,
            PeerState::Revoked
        );

        // A hostile or damaged entry prevents partial trusted presentation.
        let mut damaged = encode_record(&local, &remote, PeerState::Confirmed);
        windows_store::write_fixed(&damaged_target, &mut damaged).unwrap();
        assert!(matches!(
            list_named(&prefix, &local),
            Err(IdentityError::CorruptCredential)
        ));
    }
}
