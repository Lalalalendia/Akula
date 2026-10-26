//! User-authorized S3 vault genesis and pinned-trust transfer.
//!
//! A new remote keyspace is initialized exactly once, with provider CAS.
//! An existing root is NEVER silently claimed. The encrypted invitation
//! authenticates the genesis fingerprint from the first trusted device;
//! a second device checks the entire immutable ancestry to that genesis.

use super::{
    AwsS3ProbeClient, OperationId, S3ContractError, S3HeadError,
    S3HeadJournal, S3HeadPointerV1, S3HeadPublication, S3Keyspace,
    S3ManifestError, S3ManifestV1, S3VerifiedHead, VaultId,
};

const MAX_PAIR_ANCESTORS: usize = 128;

#[derive(Debug)]
pub enum S3PairBootstrapError {
    ExistingRemoteRoot,
    MissingRemoteRoot,
    ForeignNamespace,
    UntrustedGenesis,
    TooManyAncestors,
    Contract(S3ContractError),
    Manifest(S3ManifestError),
    Head(S3HeadError),
}

impl From<S3ContractError> for S3PairBootstrapError {
    fn from(v: S3ContractError) -> Self { Self::Contract(v) }
}
impl From<S3ManifestError> for S3PairBootstrapError {
    fn from(v: S3ManifestError) -> Self { Self::Manifest(v) }
}
impl From<S3HeadError> for S3PairBootstrapError {
    fn from(v: S3HeadError) -> Self { Self::Head(v) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct S3PairGenesis {
    pub cloud_vault_id: VaultId,
    pub genesis_fingerprint: String,
}

fn fingerprint(pointer: &S3HeadPointerV1, ns: &S3Keyspace)
    -> Result<String, S3PairBootstrapError>
{
    let encoded = pointer.encode(ns)?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

/// Requires an explicit user-authorized first-device operation and a
/// journal held in registered application-private storage outside the vault.
/// The second computer NEVER calls this; it imports and verifies the pin.
pub async fn bootstrap_new_remote_vault(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    journal: &mut S3HeadJournal,
) -> Result<S3PairGenesis, S3PairBootstrapError> {
    if client.bound_keyspace() != ns {
        return Err(S3PairBootstrapError::ForeignNamespace);
    }
    if client.read_verified_head().await?.is_some() {
        return Err(S3PairBootstrapError::ExistingRemoteRoot);
    }
    let generation = OperationId::new();
    let manifest = S3ManifestV1::new(ns, generation, 1, None, vec![])?;
    let digest = client.store_manifest_object(generation, &manifest.encode(ns)?).await?;
    let pointer = S3HeadPointerV1::new(ns, 1, generation, &digest, None)?;
    // A CAS race or lost ACK must not be reinterpreted as success.
    match client.publish_head_cas(journal, generation, pointer.clone(), None).await? {
        S3HeadPublication::Published | S3HeadPublication::Recovered => {}
    }
    let Some(verified) = client.read_verified_head().await? else {
        return Err(S3PairBootstrapError::MissingRemoteRoot);
    };
    if verified.pointer != pointer {
        return Err(S3PairBootstrapError::UntrustedGenesis);
    }
    Ok(S3PairGenesis {
        cloud_vault_id: ns.vault_id(),
        genesis_fingerprint: fingerprint(&pointer, ns)?,
    })
}

/// A successful ordinary HEAD is NOT sufficient: the root can have advanced
/// after the invitation was exported. Walk the entire verified predecessor
/// chain, reconstruct generation one's exact pointer, compare its BLAKE3 pin.
pub async fn verify_invited_genesis(
    client: &AwsS3ProbeClient,
    ns: &S3Keyspace,
    expected_fingerprint: &str,
) -> Result<S3VerifiedHead, S3PairBootstrapError> {
    if client.bound_keyspace() != ns {
        return Err(S3PairBootstrapError::ForeignNamespace);
    }
    if expected_fingerprint.len() != 64 || !expected_fingerprint.bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(S3PairBootstrapError::UntrustedGenesis);
    }
    let head = client.read_verified_head().await?
        .ok_or(S3PairBootstrapError::MissingRemoteRoot)?;
    client.verify_manifest_ancestors(&head, MAX_PAIR_ANCESTORS).await?;
    let mut cursor = head.manifest.clone();
    let mut hops = 0;
    while let (Some(parent_id), Some(parent_digest)) = (
        cursor.previous_manifest_id,
        cursor.previous_manifest_blake3.as_deref(),
    ) {
        if hops >= MAX_PAIR_ANCESTORS {
            return Err(S3PairBootstrapError::TooManyAncestors);
        }
        let previous = client.read_verified_manifest(parent_id, parent_digest).await?;
        if !cursor.is_successor_of(&previous, parent_digest) {
            return Err(S3PairBootstrapError::UntrustedGenesis);
        }
        cursor = previous;
        hops += 1;
    }
    if cursor.generation != 1 {
        return Err(S3PairBootstrapError::UntrustedGenesis);
    }
    let genesis = S3HeadPointerV1::new(
        ns, 1, cursor.manifest_id, &cursor.digest(ns)?, None,
    )?;
    if fingerprint(&genesis, ns)? != expected_fingerprint {
        return Err(S3PairBootstrapError::UntrustedGenesis);
    }
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_genesis_identity_depends_on_cloud_vault_and_manifest() {
        let ns = S3Keyspace::new("akula-sync/pair", VaultId::new()).unwrap();
        let id = OperationId::new();
        let genesis = S3ManifestV1::new(&ns, id, 1, None, vec![]).unwrap();
        let p = S3HeadPointerV1::new(
            &ns, 1, id, &genesis.digest(&ns).unwrap(), None,
        ).unwrap();
        let expected = fingerprint(&p, &ns).unwrap();
        assert_eq!(expected.len(), 64);
        let different = S3Keyspace::new("akula-sync/pair", VaultId::new()).unwrap();
        assert_ne!(expected, fingerprint(
            &S3HeadPointerV1::new(&different, 1, id,
                &genesis.digest(&ns).unwrap(), None).unwrap(), &different,
        ).unwrap());
    }
}
