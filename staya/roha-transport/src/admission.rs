//! Optional live admission for already authenticated Iroh EndpointIds.
//! The external check belongs to the Akula host, never to a network payload.
use iroh::EndpointId;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

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
