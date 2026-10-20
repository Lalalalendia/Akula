//! RN-4E1b-03c0 — a non-authorizing P2P policy-state experiment.
//! NO production installer witness, persistent state, authenticated QUIC
//! connection, File Gate writer, response receipt or Allow variant exists.
//! Caller-provided remote identity is only a claim, never channel proof.
//! A cold restart starts EMPTY and therefore refuses every old claim.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct VaultId([u8; 16]);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct ReplicaId([u8; 16]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DeviceId([u8; 16]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EndpointKey([u8; 32]);
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PinnedRoot {
    volume: [u8; 16],
    file: [u8; 16],
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Object {
    blake3: [u8; 32],
    length: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Scope {
    vault: VaultId,
    replica: ReplicaId,
    physical_root: PinnedRoot,
    device: DeviceId,
    approved_endpoint: EndpointKey,
    issuer_key: [u8; 32],
    object: Object,
    expires_at: u64,
}
impl Scope {
    fn valid(self) -> bool {
        self.vault.0 != [0; 16]
            && self.replica.0 != [0; 16]
            && self.physical_root.volume != [0; 16]
            && self.physical_root.file != [0; 16]
            && self.device.0 != [0; 16]
            && self.approved_endpoint.0 != [0; 32]
            && self.issuer_key != [0; 32]
            && self.object.blake3 != [0; 32]
            && self.expires_at > 0
    }
}
#[derive(Clone, Copy, Debug)]
struct Entry {
    scope: Scope,
    epoch: u64,
    revoked: bool,
}

/// A sibling module cannot forge this witness: its field is private.
/// Its ONLY constructor compiles under unit tests. No runtime installer
/// attestation is claimed. Future production initialization requires a new
/// independently verified SCM/Broker-owned source of authority.
pub(crate) struct InstalledAuthority {
    _private: (),
}
impl InstalledAuthority {
    #[cfg(test)]
    fn fixture() -> Self {
        Self { _private: () }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    BadScope,
    UnknownReplica,
    ReusedPhysicalRoot,
    ChangedVaultOrRoot,
    WrongOrAbsentCasEpoch,
    EpochOverflow,
    StaleEpoch,
    Revoked,
    Expired,
    DifferentPeerOrObject,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[must_use]
pub(crate) enum Observation {
    Hold(Refusal),
    /// NOT a grant: arbitrary application bytes can reproduce all these
    /// identifiers without owning the claimed live QUIC connection.
    ConsistentOnlyNoAuthenticatedConnection,
}

/// No filesystem persistence or network interface. The installed product
/// does not construct or call this laboratory kernel.
#[derive(Default)]
pub(crate) struct BrokerPolicyKernel {
    entries: BTreeMap<ReplicaId, Entry>,
}
impl BrokerPolicyKernel {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn enroll(
        &mut self,
        _installer: &InstalledAuthority,
        scope: Scope,
        compare_epoch: Option<u64>,
    ) -> Result<u64, Refusal> {
        if !scope.valid() {
            return Err(Refusal::BadScope);
        }
        if self.entries.iter().any(|(id, record)| {
            *id != scope.replica && record.scope.physical_root == scope.physical_root
        }) {
            return Err(Refusal::ReusedPhysicalRoot);
        }
        let epoch = match self.entries.get(&scope.replica) {
            None => {
                if compare_epoch.is_some() {
                    return Err(Refusal::WrongOrAbsentCasEpoch);
                }
                1
            }
            Some(record) => {
                if record.scope.vault != scope.vault
                    || record.scope.physical_root != scope.physical_root
                {
                    return Err(Refusal::ChangedVaultOrRoot);
                }
                if compare_epoch != Some(record.epoch) {
                    return Err(Refusal::WrongOrAbsentCasEpoch);
                }
                record.epoch.checked_add(1).ok_or(Refusal::EpochOverflow)?
            }
        };
        self.entries.insert(
            scope.replica,
            Entry {
                scope,
                epoch,
                revoked: false,
            },
        );
        Ok(epoch)
    }

    /// An actual durable broker-owned monotonic journal does NOT yet exist.
    pub(crate) fn revoke(
        &mut self,
        _installer: &InstalledAuthority,
        replica: ReplicaId,
        compare_epoch: u64,
    ) -> Result<u64, Refusal> {
        let record = self
            .entries
            .get_mut(&replica)
            .ok_or(Refusal::UnknownReplica)?;
        if record.epoch != compare_epoch {
            return Err(Refusal::StaleEpoch);
        }
        let next = record.epoch.checked_add(1).ok_or(Refusal::EpochOverflow)?;
        record.epoch = next;
        record.revoked = true;
        Ok(next)
    }

    /// A matching claimed EndpointKey is not proof of a transport-level
    /// authenticated peer. This function CANNOT return an access permit.
    pub(crate) fn inspect_claim(&self, claim: Scope, epoch: u64, trusted_now: u64) -> Observation {
        let Some(record) = self.entries.get(&claim.replica) else {
            return Observation::Hold(Refusal::UnknownReplica);
        };
        if record.epoch != epoch {
            return Observation::Hold(Refusal::StaleEpoch);
        }
        if record.revoked {
            return Observation::Hold(Refusal::Revoked);
        }
        if trusted_now >= record.scope.expires_at {
            return Observation::Hold(Refusal::Expired);
        }
        if record.scope != claim {
            return Observation::Hold(Refusal::DifferentPeerOrObject);
        }
        Observation::ConsistentOnlyNoAuthenticatedConnection
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(replica: u8, root: u8, object: u8) -> Scope {
        Scope {
            vault: VaultId([1; 16]),
            replica: ReplicaId([replica; 16]),
            physical_root: PinnedRoot {
                volume: [2; 16],
                file: [root; 16],
            },
            device: DeviceId([3; 16]),
            approved_endpoint: EndpointKey([4; 32]),
            issuer_key: [5; 32],
            object: Object {
                blake3: [object; 32],
                length: 128,
            },
            expires_at: 1_000,
        }
    }

    #[test]
    fn cold_restart_and_empty_policy_refuse_prior_claims() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        assert_eq!(
            k.inspect_claim(a, 1, 100),
            Observation::Hold(Refusal::UnknownReplica)
        );
        assert_eq!(k.enroll(&authority, a, None), Ok(1));
        assert_eq!(
            k.inspect_claim(a, 1, 100),
            Observation::ConsistentOnlyNoAuthenticatedConnection
        );
        let restarted = BrokerPolicyKernel::new();
        assert_eq!(
            restarted.inspect_claim(a, 1, 100),
            Observation::Hold(Refusal::UnknownReplica)
        );
    }

    #[test]
    fn two_replicas_share_remote_vault_but_not_physical_root() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        let b = scope(9, 10, 11);
        assert_eq!(a.vault, b.vault);
        assert_eq!(a.device, b.device);
        assert_eq!(a.approved_endpoint, b.approved_endpoint);
        assert_eq!(k.enroll(&authority, a, None), Ok(1));
        assert_eq!(k.enroll(&authority, b, None), Ok(1));
        assert_eq!(
            k.inspect_claim(a, 1, 100),
            Observation::ConsistentOnlyNoAuthenticatedConnection
        );
        assert_eq!(
            k.inspect_claim(b, 1, 100),
            Observation::ConsistentOnlyNoAuthenticatedConnection
        );
    }

    #[test]
    fn cross_replica_object_and_peer_substitution_refuses() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        assert_eq!(k.enroll(&authority, a, None), Ok(1));
        let mut forged = a;
        forged.physical_root = scope(9, 10, 11).physical_root;
        assert_eq!(
            k.inspect_claim(forged, 1, 100),
            Observation::Hold(Refusal::DifferentPeerOrObject)
        );
        forged = a;
        forged.object = scope(9, 10, 11).object;
        assert_eq!(
            k.inspect_claim(forged, 1, 100),
            Observation::Hold(Refusal::DifferentPeerOrObject)
        );
        forged = a;
        forged.approved_endpoint = EndpointKey([77; 32]);
        assert_eq!(
            k.inspect_claim(forged, 1, 100),
            Observation::Hold(Refusal::DifferentPeerOrObject)
        );
    }

    #[test]
    fn duplicate_physical_root_and_identity_rebinding_refuse() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        assert_eq!(k.enroll(&authority, a, None), Ok(1));
        assert_eq!(
            k.enroll(&authority, scope(9, 7, 11), None),
            Err(Refusal::ReusedPhysicalRoot)
        );
        let mut changed = a;
        changed.vault = VaultId([8; 16]);
        assert_eq!(
            k.enroll(&authority, changed, Some(1)),
            Err(Refusal::ChangedVaultOrRoot)
        );
    }

    #[test]
    fn revoked_epoch_cannot_be_replayed_or_silently_reissued() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        assert_eq!(k.enroll(&authority, a, None), Ok(1));
        assert_eq!(
            k.enroll(&authority, a, None),
            Err(Refusal::WrongOrAbsentCasEpoch)
        );
        assert_eq!(k.revoke(&authority, a.replica, 1), Ok(2));
        assert_eq!(
            k.inspect_claim(a, 1, 100),
            Observation::Hold(Refusal::StaleEpoch)
        );
        assert_eq!(
            k.inspect_claim(a, 2, 100),
            Observation::Hold(Refusal::Revoked)
        );
        assert_eq!(
            k.enroll(&authority, a, Some(1)),
            Err(Refusal::WrongOrAbsentCasEpoch)
        );
        assert_eq!(k.enroll(&authority, a, Some(2)), Ok(3));
        assert_eq!(
            k.inspect_claim(a, 2, 100),
            Observation::Hold(Refusal::StaleEpoch)
        );
        assert_eq!(
            k.inspect_claim(a, 3, 100),
            Observation::ConsistentOnlyNoAuthenticatedConnection
        );
    }

    #[test]
    fn expired_policy_and_overflow_fail_closed() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        assert_eq!(k.enroll(&authority, a, None), Ok(1));
        assert_eq!(
            k.inspect_claim(a, 1, 1_000),
            Observation::Hold(Refusal::Expired)
        );
        let entry = k.entries.get_mut(&a.replica).unwrap();
        entry.epoch = u64::MAX;
        entry.revoked = true;
        assert_eq!(
            k.revoke(&authority, a.replica, u64::MAX),
            Err(Refusal::EpochOverflow)
        );
        assert_eq!(
            k.enroll(&authority, a, Some(u64::MAX)),
            Err(Refusal::EpochOverflow)
        );
        assert_eq!(
            k.inspect_claim(a, u64::MAX, 100),
            Observation::Hold(Refusal::Revoked)
        );
    }

    #[test]
    fn structurally_invalid_scope_and_unknown_cas_epoch_refuse() {
        let authority = InstalledAuthority::fixture();
        let mut k = BrokerPolicyKernel::new();
        let a = scope(6, 7, 8);
        let mut empty = a;
        empty.approved_endpoint = EndpointKey([0; 32]);
        assert_eq!(k.enroll(&authority, empty, None), Err(Refusal::BadScope));
        assert_eq!(
            k.enroll(&authority, a, Some(0)),
            Err(Refusal::WrongOrAbsentCasEpoch)
        );
        assert_eq!(
            k.inspect_claim(a, 0, 100),
            Observation::Hold(Refusal::UnknownReplica)
        );
    }
}
