//! Node V1 local process boundary. No public listener or installer authority.
//! Authenticated transport, signed package activation and remote Put/Get must
//! remain disabled until the separate release-wide E2E gate accepts them.

mod fence;
mod identity;
mod put;
mod receipt;
mod store;
mod trust;

pub use fence::DurableChallengeFence;
pub use trust::{AdmittedPut, AdmittedRead, RootTrust};
pub use identity::NodeIdentity;
pub use put::{OwnerPutGrant, MAX_PUT_BYTES, put_claim, put_proof_claim, signed_put_digest};
pub use receipt::NodePutReceipt;

use std::io;
use std::path::{Path, PathBuf};
use crate::node_replica_auth_lab::{
    DeviceReadProof, OwnerReadGrant, device_challenge_claim,
};
use crate::sync::device_roster_lab::OwnerSignedRoster;
use ring::signature::{UnparsedPublicKey, ED25519};

#[derive(Debug)]
pub enum NodeV1Error {
    Io(io::Error),
    Sqlite(rusqlite::Error),
    Hold(&'static str),
}
impl From<io::Error> for NodeV1Error {
    fn from(value: io::Error) -> Self { Self::Io(value) }
}
impl From<rusqlite::Error> for NodeV1Error {
    fn from(value: rusqlite::Error) -> Self { Self::Sqlite(value) }
}

pub struct NodeHost {
    identity: NodeIdentity,
    trust: RootTrust,
    ledger: DurableChallengeFence,
    state: PathBuf,
}

impl NodeHost {
    /// An actual installed service has a fixed StateDirectory and an
    /// independently root-enrolled policy. Never start this service as root.
    pub fn open_provisioned() -> Result<Self, NodeV1Error> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Err(NodeV1Error::Hold("Node must not run as root"));
        }
        let state = Path::new("/var/lib/akula-node");
        let identity = NodeIdentity::open_provisioned(state)?;
        let trust = RootTrust::open_provisioned(identity.public_key())?;
        let ledger = DurableChallengeFence::open(state, trust.clone())?;
        Ok(Self { identity, trust, ledger, state: state.to_path_buf() })
    }

    /// Only for explicitly feature-gated debug integration E2E. No shipped
    /// binary can redirect root trust to an application-owned policy.
    #[cfg(all(feature = "node-v1-transport-tests", debug_assertions))]
    pub fn open_integration_fixture(
        state: &Path, policy_dir: &Path,
    ) -> Result<Self, NodeV1Error> {
        let identity = NodeIdentity::open_or_create(state)?;
        let trust = RootTrust::open_integration_fixture(policy_dir, identity.public_key())?;
        let ledger = DurableChallengeFence::open(state, trust.clone())?;
        Ok(Self { identity, trust, ledger, state: state.to_path_buf() })
    }

    pub fn public_key(&self) -> [u8; 32] { self.identity.public_key() }

    #[cfg(all(feature = "node-v1-transport-tests", debug_assertions))]
    pub fn mail_operator_key_for_lab(&self) -> Result<[u8; 32], NodeV1Error> {
        Ok(self.trust.witnessed_roster()?.owner_public_key)
    }

    pub fn verify_iroh_endpoint(&self, endpoint: [u8; 32])
        -> Result<(), NodeV1Error>
    {
        self.trust.require_iroh_endpoint(endpoint)
    }

    pub fn sign_local_health(&self, challenge: [u8; 32])
        -> Result<[u8; 64], NodeV1Error>
    {
        self.trust.require_current()?;
        self.identity.sign_health(challenge)
    }

    /// A one-use ledger is a gate, never a replacement for owner signatures,
    /// an authenticated Iroh remote_id, or a current independent root policy.
    pub fn challenge_fence(&mut self) -> &mut DurableChallengeFence {
        &mut self.ledger
    }

    /// The caller must supply the peer key from a completed, authenticated
    /// Iroh QUIC connection. This local Core method does not open a socket.
    pub fn issue_put_challenge(
        &mut self, roster: &OwnerSignedRoster, grant: &OwnerPutGrant,
        authenticated_peer: [u8; 32],
    ) -> Result<[u8; 32], NodeV1Error> {
        let scope = self.trust.admit_put(roster, grant, authenticated_peer)?;
        self.ledger.issue_put(&scope)
    }

    /// An exact signed Put can publish only after the issued challenge was
    /// proven by its enrolled device. A failed stage never gets a receipt.
    pub fn commit_put(
        &mut self, roster: &OwnerSignedRoster, grant: &OwnerPutGrant,
        authenticated_peer: [u8; 32], challenge: [u8; 32],
        device_signature: [u8; 64], bytes: &[u8],
    ) -> Result<NodePutReceipt, NodeV1Error> {
        let admission = self.trust.admit_put(roster, grant, authenticated_peer)?;
        put::verify_put_proof(grant, challenge, &device_signature)
            .map_err(|_| NodeV1Error::Hold("invalid device Put proof"))?;
        if !self.ledger.consume_put(&admission, challenge)? {
            return Err(NodeV1Error::Hold("Put challenge missing, expired or already consumed"));
        }
        let revision = store::commit(&self.state, &admission, bytes)?;
        self.trust.require_current()?;
        receipt::sign_local_receipt(
            &self.identity, revision, grant.grant_epoch, admission.digest(),
        )
    }

    pub fn issue_read_challenge(
        &mut self, roster: &OwnerSignedRoster, grant: &OwnerReadGrant,
        authenticated_peer: [u8; 32],
    ) -> Result<[u8; 32], NodeV1Error> {
        let admission = self.trust.admit_read(roster, grant, authenticated_peer)?;
        self.ledger.issue(&admission)
    }

    /// A read challenge never authorizes a write. Only this exact grant's
    /// immutable DAG revision is disclosed, and never to an unverified peer.
    pub fn fulfill_read(
        &mut self, roster: &OwnerSignedRoster, grant: &OwnerReadGrant,
        authenticated_peer: [u8; 32], challenge: [u8; 32],
        proof: &DeviceReadProof,
    ) -> Result<Vec<u8>, NodeV1Error> {
        let admission = self.trust.admit_read(roster, grant, authenticated_peer)?;
        let claim = device_challenge_claim(grant, challenge)
            .map_err(|_| NodeV1Error::Hold("invalid read proof claim"))?;
        UnparsedPublicKey::new(&ED25519, grant.device_public_key)
            .verify(&claim, &proof.device_signature)
            .map_err(|_| NodeV1Error::Hold("invalid device read proof"))?;
        if !self.ledger.consume_once(
            authenticated_peer, admission.grant_digest(), challenge,
        )? {
            return Err(NodeV1Error::Hold("Read challenge missing, expired or consumed"));
        }
        let scope = store::StoredRevision {
            vault_id: grant.vault_id,
            document_id: grant.document_id,
            revision_id: grant.revision_id,
            raw_blake3: grant.expected_blake3.clone(),
            byte_length: 0,
        };
        let bytes = store::read_verified(&self.state, &scope, &grant.filename)?;
        self.trust.require_current()?;
        Ok(bytes)
    }
}
