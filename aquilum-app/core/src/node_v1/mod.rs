//! Node V1 local process boundary. No public listener or installer authority.
//! Authenticated transport, signed package activation and remote Put/Get must
//! remain disabled until the separate release-wide E2E gate accepts them.

mod fence;
mod identity;
mod trust;

pub use fence::DurableChallengeFence;
pub use trust::{AdmittedRead, RootTrust};
pub use identity::NodeIdentity;

use std::io;
use std::path::Path;

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
}

impl NodeHost {
    /// An actual installed service has a fixed StateDirectory and an
    /// independently root-enrolled policy. Never start this service as root.
    pub fn open_provisioned() -> Result<Self, NodeV1Error> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Err(NodeV1Error::Hold("Node must not run as root"));
        }
        let state = Path::new("/var/lib/akula-node");
        let identity = NodeIdentity::open_or_create(state)?;
        let trust = RootTrust::open_provisioned(identity.public_key())?;
        let ledger = DurableChallengeFence::open(state, trust.clone())?;
        Ok(Self { identity, trust, ledger })
    }

    pub fn public_key(&self) -> [u8; 32] { self.identity.public_key() }

    /// This proves the local process owns the enrolled Node key. It does
    /// not prove remote connectivity, a writable store or a healthy data plane.
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
}
