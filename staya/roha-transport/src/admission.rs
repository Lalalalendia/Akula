//! Research-only preflight budgeting for an already-authorized P2P staging area.
//!
//! NOT a physical filesystem quota. Limits trusted declared object sizes,
//! active leases and concurrent transfers before QUIC/iroh-blobs fetches.
//! A remote-advertised size is NOT a trusted length. The Akula Sync Core must
//! authorize EndpointId/DeviceId/VaultId and immutable BLAKE3+length first.
//!
//! Every retained temporary file must keep its lease until the backing data
//! is deleted. This in-memory ledger loses accounting across process restarts
//! and excludes database, Bao and filesystem overhead. Before production use
//! add a restart-safe ledger and an OS-enforced hard disk quota.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use iroh::EndpointId;

/// Nonzero policy supplied locally, never by an untrusted remote peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionLimits {
    pub max_object_bytes: u64,
    pub max_tracked_bytes: u64,
    pub max_peer_bytes: u64,
    pub max_active: usize,
    pub max_peer_active: usize,
}

impl AdmissionLimits {
    pub fn validate(self) -> Result<Self, AdmissionError> {
        if self.max_object_bytes == 0
            || self.max_tracked_bytes == 0
            || self.max_peer_bytes == 0
            || self.max_active == 0
            || self.max_peer_active == 0
            || self.max_object_bytes > self.max_tracked_bytes
            || self.max_object_bytes > self.max_peer_bytes
            || self.max_peer_bytes > self.max_tracked_bytes
            || self.max_peer_active > self.max_active
        {
            return Err(AdmissionError::InvalidLimits);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    InvalidLimits,
    ObjectTooLarge,
    DuplicateObject,
    GlobalSlotsExhausted,
    PeerSlotsExhausted,
    GlobalBytesExhausted,
    PeerBytesExhausted,
    VerificationFailed,
    LedgerUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub tracked_objects: usize,
    pub tracked_bytes: u64,
    pub verified_objects: usize,
}

struct Slot {
    peer: EndpointId,
    expected_bytes: u64,
    verified: bool,
}

#[derive(Default)]
struct State {
    tracked_bytes: u64,
    objects: BTreeMap<[u8; 32], Slot>,
}

struct Shared {
    limits: AdmissionLimits,
    state: Mutex<State>,
}

/// Cloneable handle to a process-local fail-closed ledger.
#[derive(Clone)]
pub struct StageBudget {
    shared: Arc<Shared>,
}

/// Non-cloneable lease that accounts for both in-flight and retained bytes.
/// Dropping it releases the ledger ONLY, not the FsStore physical data.
pub struct StageLease {
    shared: Arc<Shared>,
    hash: [u8; 32],
    expected_bytes: u64,
}

impl StageBudget {
    pub fn new(limits: AdmissionLimits) -> Result<Self, AdmissionError> {
        Ok(Self {
            shared: Arc::new(Shared {
                limits: limits.validate()?,
                state: Mutex::new(State::default()),
            }),
        })
    }

    /// Call before a network request or FsStore writer. Hash and size MUST
    /// originate in independently authorized, verified revision metadata.
    pub fn reserve(
        &self,
        peer: EndpointId,
        hash: [u8; 32],
        expected_bytes: u64,
    ) -> Result<StageLease, AdmissionError> {
        let limits = self.shared.limits;
        if expected_bytes > limits.max_object_bytes {
            return Err(AdmissionError::ObjectTooLarge);
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| AdmissionError::LedgerUnavailable)?;
        if state.objects.contains_key(&hash) {
            return Err(AdmissionError::DuplicateObject);
        }
        if state.objects.len() >= limits.max_active {
            return Err(AdmissionError::GlobalSlotsExhausted);
        }
        let mut peer_count = 0usize;
        let mut peer_bytes = 0u64;
        for slot in state.objects.values().filter(|s| s.peer == peer) {
            peer_count = peer_count
                .checked_add(1)
                .ok_or(AdmissionError::PeerSlotsExhausted)?;
            peer_bytes = peer_bytes
                .checked_add(slot.expected_bytes)
                .ok_or(AdmissionError::PeerBytesExhausted)?;
        }
        if peer_count >= limits.max_peer_active {
            return Err(AdmissionError::PeerSlotsExhausted);
        }
        if state
            .tracked_bytes
            .checked_add(expected_bytes)
            .filter(|n| *n <= limits.max_tracked_bytes)
            .is_none()
        {
            return Err(AdmissionError::GlobalBytesExhausted);
        }
        if peer_bytes
            .checked_add(expected_bytes)
            .filter(|n| *n <= limits.max_peer_bytes)
            .is_none()
        {
            return Err(AdmissionError::PeerBytesExhausted);
        }
        state.tracked_bytes += expected_bytes;
        state.objects.insert(
            hash,
            Slot {
                peer,
                expected_bytes,
                verified: false,
            },
        );
        Ok(StageLease {
            shared: Arc::clone(&self.shared),
            hash,
            expected_bytes,
        })
    }

    pub fn snapshot(&self) -> Result<Snapshot, AdmissionError> {
        let state = self
            .shared
            .state
            .lock()
            .map_err(|_| AdmissionError::LedgerUnavailable)?;
        Ok(Snapshot {
            tracked_objects: state.objects.len(),
            tracked_bytes: state.tracked_bytes,
            verified_objects: state.objects.values().filter(|s| s.verified).count(),
        })
    }
}

impl StageLease {
    /// Only exact full-size BLAKE3 bytes count as verified. This is NOT a
    /// File Gate publication permit; caller must retain lease until cleanup.
    pub fn verify(&mut self, full_bytes: &[u8]) -> Result<(), AdmissionError> {
        let len = u64::try_from(full_bytes.len())
            .map_err(|_| AdmissionError::VerificationFailed)?;
        if len != self.expected_bytes || *blake3::hash(full_bytes).as_bytes() != self.hash {
            return Err(AdmissionError::VerificationFailed);
        }
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| AdmissionError::LedgerUnavailable)?;
        let slot = state
            .objects
            .get_mut(&self.hash)
            .ok_or(AdmissionError::LedgerUnavailable)?;
        slot.verified = true;
        Ok(())
    }
}

impl Drop for StageLease {
    fn drop(&mut self) {
        // Poisoning fails future admissions closed but our own lease must
        // still be removed. Never panic from Drop during cancellation.
        let mut state = match self.shared.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(slot) = state.objects.remove(&self.hash) {
            state.tracked_bytes = state.tracked_bytes.saturating_sub(slot.expected_bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use std::{
        sync::{Arc, Barrier},
        thread,
    };

    fn peer() -> EndpointId {
        SecretKey::generate().public()
    }

    fn example_limits() -> AdmissionLimits {
        AdmissionLimits {
            max_object_bytes: 1024,
            max_tracked_bytes: 2048,
            max_peer_bytes: 1024,
            max_active: 3,
            max_peer_active: 2,
        }
    }

    fn item(n: u8) -> [u8; 32] {
        *blake3::hash(&[n]).as_bytes()
    }

    #[test]
    fn invalid_and_zero_limits_are_rejected() {
        let x = example_limits();
        for bad in [
            AdmissionLimits { max_object_bytes: 0, ..x },
            AdmissionLimits { max_tracked_bytes: 0, ..x },
            AdmissionLimits { max_peer_bytes: 0, ..x },
            AdmissionLimits { max_active: 0, ..x },
            AdmissionLimits { max_peer_active: 0, ..x },
            AdmissionLimits { max_object_bytes: 3000, ..x },
            AdmissionLimits { max_peer_bytes: 3000, ..x },
            AdmissionLimits { max_peer_active: 4, ..x },
        ] {
            assert!(matches!(StageBudget::new(bad), Err(AdmissionError::InvalidLimits)));
        }
    }

    #[test]
    fn oversize_and_integer_overflow_refused_without_mutating_state() {
        let budget = StageBudget::new(example_limits()).unwrap();
        let user = peer();
        for bytes in [1025, u64::MAX] {
            assert!(matches!(
                budget.reserve(user, item(1), bytes),
                Err(AdmissionError::ObjectTooLarge)
            ));
        }
        assert_eq!(budget.snapshot().unwrap().tracked_bytes, 0);
    }

    #[test]
    fn duplicate_hash_and_allocation_limits_are_atomic() {
        let budget = StageBudget::new(example_limits()).unwrap();
        let a = peer();
        let b = peer();
        let first = budget.reserve(a, item(1), 1024).unwrap();
        assert!(matches!(
            budget.reserve(b, item(1), 1),
            Err(AdmissionError::DuplicateObject)
        ));
        let second = budget.reserve(b, item(2), 1024).unwrap();
        assert!(matches!(
            budget.reserve(a, item(3), 1),
            Err(AdmissionError::GlobalBytesExhausted)
        ));
        assert_eq!(
            budget.snapshot().unwrap(),
            Snapshot {
                tracked_objects: 2,
                tracked_bytes: 2048,
                verified_objects: 0,
            }
        );
        drop(first);
        let third = budget.reserve(a, item(3), 1024).unwrap();
        assert!(matches!(
            budget.reserve(b, item(4), 1),
            Err(AdmissionError::GlobalBytesExhausted)
        ));
        drop((second, third));
        assert_eq!(budget.snapshot().unwrap().tracked_bytes, 0);
    }

    #[test]
    fn per_peer_limit_and_global_slots_work_independently() {
        let limits = AdmissionLimits {
            max_object_bytes: 500,
            max_tracked_bytes: 4000,
            max_peer_bytes: 700,
            max_active: 3,
            max_peer_active: 2,
        };
        let budget = StageBudget::new(limits).unwrap();
        let a = peer();
        let b = peer();
        let _one = budget.reserve(a, item(1), 400).unwrap();
        assert!(matches!(
            budget.reserve(a, item(2), 350),
            Err(AdmissionError::PeerBytesExhausted)
        ));
        let _two = budget.reserve(a, item(2), 250).unwrap();
        assert!(matches!(
            budget.reserve(a, item(3), 1),
            Err(AdmissionError::PeerSlotsExhausted)
        ));
        let _three = budget.reserve(b, item(3), 400).unwrap();
        assert!(matches!(
            budget.reserve(peer(), item(4), 1),
            Err(AdmissionError::GlobalSlotsExhausted)
        ));
    }

    #[test]
    fn wrong_target_never_promotes_and_verified_object_retains_capacity() {
        let budget = StageBudget::new(example_limits()).unwrap();
        let user = peer();
        let body = b"verified exact content";
        let expected = *blake3::hash(body).as_bytes();
        let mut lease = budget.reserve(user, expected, body.len() as u64).unwrap();
        assert_eq!(lease.verify(b"changed exact content"), Err(AdmissionError::VerificationFailed));
        assert_eq!(budget.snapshot().unwrap().verified_objects, 0);
        assert_eq!(lease.verify(body), Ok(()));
        assert_eq!(budget.snapshot().unwrap().verified_objects, 1);
        assert_eq!(budget.snapshot().unwrap().tracked_bytes, body.len() as u64);
        drop(lease);
        assert_eq!(budget.snapshot().unwrap().tracked_bytes, 0);
    }

    #[test]
    fn cancel_and_panic_release_reservations() {
        let budget = StageBudget::new(example_limits()).unwrap();
        let user = peer();
        let reserve = budget.reserve(user, item(1), 512).unwrap();
        drop(reserve);
        let panicked = std::panic::catch_unwind(|| {
            let _temp = budget.reserve(user, item(2), 512).unwrap();
            panic!("cancel downloader before commit");
        });
        assert!(panicked.is_err());
        assert_eq!(budget.snapshot().unwrap().tracked_bytes, 0);
        assert_eq!(budget.snapshot().unwrap().tracked_objects, 0);
    }

    #[test]
    fn simultaneous_requests_cannot_reserve_more_than_global_slots() {
        const WORKERS: usize = 12;
        let budget = StageBudget::new(AdmissionLimits {
            max_object_bytes: 1024,
            max_tracked_bytes: 3072,
            max_peer_bytes: 3072,
            max_active: 3,
            max_peer_active: 3,
        }).unwrap();
        let user = peer();
        let start = Arc::new(Barrier::new(WORKERS));
        let finish = Arc::new(Barrier::new(WORKERS));
        let mut threads = Vec::new();
        for i in 0..WORKERS {
            let budget = budget.clone();
            let user = user;
            let start = Arc::clone(&start);
            let finish = Arc::clone(&finish);
            threads.push(thread::spawn(move || {
                start.wait();
                let lease = budget.reserve(user, item(i as u8), 1024);
                finish.wait();
                lease
            }));
        }
        let mut successful = Vec::new();
        for handle in threads {
            if let Ok(lease) = handle.join().unwrap() {
                successful.push(lease);
            }
        }
        assert!(successful.len() <= 3);
        assert_eq!(budget.snapshot().unwrap().tracked_objects, successful.len());
        drop(successful);
        assert_eq!(budget.snapshot().unwrap().tracked_bytes, 0);
    }
}
