//! Client side of the two-stream NodeV1 protocol. Transport peer must be an
//! independently pinned exact 32-byte Iroh EndpointId. Signing is delegated
//! to the existing protected device-key provider, never a plaintext key.
use super::{
    NodeWireDenied,
    wire::{
        ChallengeReply, MAX_ACK, MAX_CONTROL, OpenRequest, Operation, WireGrant, WireReceipt,
        WireRoster,
    },
};
use aquilum_core::node_replica_auth_lab::{OwnerReadGrant, device_challenge_claim};
use aquilum_core::node_v1::{
    MAX_PUT_BYTES, NodePutReceipt, OwnerPutGrant, put_proof_claim, signed_put_digest,
};
use aquilum_core::sync::device_roster_lab::OwnerSignedRoster;
use iroh::endpoint::Connection;
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{ED25519, UnparsedPublicKey},
};
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(15);
const HEALTH_DOMAIN: &[u8] = b"Akula/NodeV1/local-health-proof/v1\0";

fn pinned_peer(conn: &Connection, expected_peer: [u8; 32]) -> Result<(), NodeWireDenied> {
    if expected_peer == [0; 32] || conn.remote_id().as_bytes() != &expected_peer {
        return Err(NodeWireDenied);
    }
    Ok(())
}

async fn start(
    conn: &Connection,
    operation: Operation,
    roster: &OwnerSignedRoster,
    grant: WireGrant,
    pinned_endpoint: [u8; 32],
    pinned_node: [u8; 32],
) -> Result<ChallengeReply, NodeWireDenied> {
    pinned_peer(conn, pinned_endpoint)?;
    if pinned_node == [0; 32] {
        return Err(NodeWireDenied);
    }
    let mut client_nonce = [0u8; 32];
    SystemRandom::new()
        .fill(&mut client_nonce)
        .map_err(|_| NodeWireDenied)?;
    let request = OpenRequest {
        version: 1,
        operation,
        client_nonce,
        roster: WireRoster::from(roster),
        grant,
    };
    let bytes = serde_json::to_vec(&request).map_err(|_| NodeWireDenied)?;
    if bytes.len() > MAX_CONTROL {
        return Err(NodeWireDenied);
    }
    let (mut send, mut recv) = tokio::time::timeout(DEADLINE, conn.open_bi())
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    tokio::time::timeout(DEADLINE, send.write_all(&bytes))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    send.finish().map_err(|_| NodeWireDenied)?;
    let answer = tokio::time::timeout(DEADLINE, recv.read_to_end(MAX_ACK))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    let reply: ChallengeReply = serde_json::from_slice(&answer).map_err(|_| NodeWireDenied)?;
    if reply.version != 1 || reply.node_public != pinned_node || reply.challenge == [0; 32] {
        return Err(NodeWireDenied);
    }
    let signature: [u8; 64] = reply
        .health_signature
        .as_slice()
        .try_into()
        .map_err(|_| NodeWireDenied)?;
    let mut claim = Vec::with_capacity(HEALTH_DOMAIN.len() + 64);
    claim.extend_from_slice(HEALTH_DOMAIN);
    claim.extend_from_slice(&pinned_node);
    claim.extend_from_slice(&client_nonce);
    UnparsedPublicKey::new(&ED25519, pinned_node)
        .verify(&claim, &signature)
        .map_err(|_| NodeWireDenied)?;
    Ok(reply)
}

async fn exchange(
    conn: &Connection,
    payload: &[u8],
    max_response: usize,
) -> Result<Vec<u8>, NodeWireDenied> {
    let (mut send, mut recv) = tokio::time::timeout(DEADLINE, conn.open_bi())
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    tokio::time::timeout(DEADLINE, send.write_all(payload))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    send.finish().map_err(|_| NodeWireDenied)?;
    tokio::time::timeout(DEADLINE, recv.read_to_end(max_response))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)
}

/// The device signer receives only a domain-separated claim and must use the
/// OS-protected Ed25519 key already enrolled by the owner. It must not accept
/// raw private keys from a node invitation or caller-controlled UI field.
pub async fn put_over_authenticated_iroh(
    conn: &Connection,
    pinned_endpoint: [u8; 32],
    roster: &OwnerSignedRoster,
    grant: &OwnerPutGrant,
    data: &[u8],
    sign: impl FnOnce(&[u8]) -> Result<[u8; 64], NodeWireDenied>,
) -> Result<NodePutReceipt, NodeWireDenied> {
    if data.len() > MAX_PUT_BYTES
        || std::str::from_utf8(data).is_err()
        || blake3::hash(data).to_hex().as_str() != grant.expected_blake3
    {
        return Err(NodeWireDenied);
    }
    let challenge = start(
        conn,
        Operation::Put,
        roster,
        WireGrant::from(grant),
        pinned_endpoint,
        grant.node_identity,
    )
    .await?;
    let claim = put_proof_claim(grant, challenge.challenge).map_err(|_| NodeWireDenied)?;
    let signature = sign(&claim)?;
    let mut payload = Vec::with_capacity(64 + data.len());
    payload.extend_from_slice(&signature);
    payload.extend_from_slice(data);
    let response = exchange(conn, &payload, MAX_ACK).await?;
    let wire: WireReceipt = serde_json::from_slice(&response).map_err(|_| NodeWireDenied)?;
    let receipt = NodePutReceipt::try_from(wire)?;
    receipt
        .verify(grant.node_identity)
        .map_err(|_| NodeWireDenied)?;
    if receipt.vault_id != grant.vault_id
        || receipt.document_id != grant.document_id
        || receipt.revision_id != grant.revision_id
        || receipt.raw_blake3 != grant.expected_blake3
        || receipt.byte_length != data.len() as u64
        || receipt.owner_epoch != grant.grant_epoch
        || receipt.put_grant_digest != signed_put_digest(grant).map_err(|_| NodeWireDenied)?
    {
        return Err(NodeWireDenied);
    }
    Ok(receipt)
}

/// Returned bytes have been both server-grant-checked and rehashed locally
/// against the exact owner-signed read grant, never a requested untrusted path.
pub async fn get_over_authenticated_iroh(
    conn: &Connection,
    pinned_endpoint: [u8; 32],
    roster: &OwnerSignedRoster,
    grant: &OwnerReadGrant,
    sign: impl FnOnce(&[u8]) -> Result<[u8; 64], NodeWireDenied>,
) -> Result<Vec<u8>, NodeWireDenied> {
    let challenge = start(
        conn,
        Operation::Get,
        roster,
        WireGrant::from(grant),
        pinned_endpoint,
        grant.node_identity,
    )
    .await?;
    let claim = device_challenge_claim(grant, challenge.challenge).map_err(|_| NodeWireDenied)?;
    let signature = sign(&claim)?;
    let bytes = exchange(conn, &signature, MAX_PUT_BYTES).await?;
    if blake3::hash(&bytes).to_hex().as_str() != grant.expected_blake3
        || std::str::from_utf8(&bytes).is_err()
    {
        return Err(NodeWireDenied);
    }
    Ok(bytes)
}
