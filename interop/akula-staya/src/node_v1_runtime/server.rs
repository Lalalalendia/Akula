//! One established, fully handshaken Iroh connection, one bounded operation.
//! The endpoint accept loop is intentionally outside this module. There is
//! no plaintext or HTTP fallback, and never an unverified peer-id parameter.
use super::{
    NodeWireDenied,
    wire::{ChallengeReply, MAX_ACK, MAX_CONTROL, OpenRequest, Operation, WireReceipt},
};
use aquilum_core::node_replica_auth_lab::DeviceReadProof;
use aquilum_core::node_v1::{MAX_PUT_BYTES, NodeHost};
use aquilum_core::sync::device_roster_lab::OwnerSignedRoster;
use iroh::endpoint::Connection;
use std::time::Duration;

const DEADLINE: Duration = Duration::from_secs(15);
const HOLD: &[u8] = b"AKULA_NODE_V1_HOLD";

/// One exact Put or Get on a completed TLS/QUIC session. No 0-RTT.
/// Connections that fail grants receive no challenge and disclose no bytes.
pub async fn serve_one_authenticated_connection(
    conn: &Connection,
    node: &mut NodeHost,
) -> Result<(), NodeWireDenied> {
    let peer = *conn.remote_id().as_bytes();
    if peer == [0; 32] {
        return Err(NodeWireDenied);
    }
    let (mut send, mut recv) = tokio::time::timeout(DEADLINE, conn.accept_bi())
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    let wire = tokio::time::timeout(DEADLINE, recv.read_to_end(MAX_CONTROL))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    let request: OpenRequest = serde_json::from_slice(&wire).map_err(|_| NodeWireDenied)?;
    if request.version != 1 || request.client_nonce == [0; 32] {
        return Err(NodeWireDenied);
    }
    let roster: OwnerSignedRoster = request.roster.try_into()?;
    let grant = request.grant;
    // Grant type is selected before authorizing the operation. Cross-type
    // domain substitution is rejected by the pinned root-owned digest.
    let challenge = match request.operation {
        Operation::Put => node
            .issue_put_challenge(&roster, &grant.clone().put()?, peer)
            .map_err(|_| NodeWireDenied)?,
        Operation::Get => node
            .issue_read_challenge(&roster, &grant.clone().read()?, peer)
            .map_err(|_| NodeWireDenied)?,
    };
    let signature = node
        .sign_local_health(request.client_nonce)
        .map_err(|_| NodeWireDenied)?;
    let response = ChallengeReply {
        version: 1,
        node_public: node.public_key(),
        health_signature: signature.to_vec(),
        challenge,
    };
    let encoded = serde_json::to_vec(&response).map_err(|_| NodeWireDenied)?;
    if encoded.len() > MAX_ACK {
        return Err(NodeWireDenied);
    }
    tokio::time::timeout(DEADLINE, send.write_all(&encoded))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    send.finish().map_err(|_| NodeWireDenied)?;

    // A second bounded bi-stream is opened only after the first message has
    // established the signed owner grant and delivered an issued challenge.
    let (mut reply, mut body_stream) = tokio::time::timeout(DEADLINE, conn.accept_bi())
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    let limit = match request.operation {
        Operation::Put => MAX_PUT_BYTES + 64,
        Operation::Get => 64,
    };
    let body = tokio::time::timeout(DEADLINE, body_stream.read_to_end(limit))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    if body.len() < 64 {
        return Err(NodeWireDenied);
    }
    let device_signature: [u8; 64] = body[..64].try_into().map_err(|_| NodeWireDenied)?;
    let response = match request.operation {
        Operation::Put => {
            let grant = grant.put()?;
            let result = node
                .commit_put(
                    &roster,
                    &grant,
                    peer,
                    challenge,
                    device_signature,
                    &body[64..],
                )
                .map_err(|_| NodeWireDenied);
            result.and_then(|receipt| {
                let encoded =
                    serde_json::to_vec(&WireReceipt::from(&receipt)).map_err(|_| NodeWireDenied)?;
                if encoded.len() > MAX_ACK {
                    return Err(NodeWireDenied);
                }
                Ok(encoded)
            })
        }
        Operation::Get => {
            if body.len() != 64 {
                return Err(NodeWireDenied);
            }
            let grant = grant.read()?;
            let proof = DeviceReadProof { device_signature };
            node.fulfill_read(&roster, &grant, peer, challenge, &proof)
                .map_err(|_| NodeWireDenied)
                .and_then(|bytes| {
                    if bytes.len() > MAX_PUT_BYTES {
                        Err(NodeWireDenied)
                    } else {
                        Ok(bytes)
                    }
                })
        }
    };
    // Never disclose whether it was absent content, a revoked grant, a stale
    // challenge, or a corrupt NodeStore. All authorization failures are opaque.
    let answer = response.as_deref().unwrap_or(HOLD);
    tokio::time::timeout(DEADLINE, reply.write_all(answer))
        .await
        .map_err(|_| NodeWireDenied)?
        .map_err(|_| NodeWireDenied)?;
    reply.finish().map_err(|_| NodeWireDenied)?;
    response.map(|_| ())
}
