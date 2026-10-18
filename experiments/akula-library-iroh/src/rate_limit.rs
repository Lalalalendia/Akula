//! In-process pacing of *outgoing application bytes*, independent of durable quotas.
//! Controls global and per-EndpointId egress simultaneously for all QUIC connections.
//! This is not a UDP/IP wire-byte cap and not a persistent daily/monthly byte limit.

use anyhow::{ensure, Result};
use iroh::EndpointId;
use std::{collections::HashMap, sync::Mutex};
use tokio::time::{sleep_until, Duration, Instant};

/// Upper bound of one eagerly sent burst before scheduling the next slice.
pub const PACE_SLICE: usize = 8 * 1024;
/// Keep a 256-KiB transfer below our bounded 60-second client read timeout.
pub const MIN_BYTES_PER_SEC: u64 = 16 * 1024;

struct Schedule {
    global_next: Instant,
    peers_next: HashMap<EndpointId, Instant>,
}

/// A conservative, shared leaky-bucket scheduler. Each reservation occurs before
/// writing any bytes, and failures never refund time already reserved.
pub struct RateGate {
    global_bps: u64,
    per_peer_bps: u64,
    schedule: Mutex<Schedule>,
}

impl RateGate {
    pub fn new(global_bps: u64, per_peer_bps: u64) -> Result<Self> {
        ensure!(global_bps >= MIN_BYTES_PER_SEC, "global rate too low (min 16384 bytes/s)");
        ensure!(per_peer_bps >= MIN_BYTES_PER_SEC, "peer rate too low (min 16384 bytes/s)");
        ensure!(per_peer_bps <= global_bps, "peer rate must not exceed global rate");
        Ok(Self {
            global_bps, per_peer_bps,
            schedule: Mutex::new(Schedule {
                global_next: Instant::now(),
                peers_next: HashMap::new(),
            }),
        })
    }

    fn reserve_at(&self, peer: EndpointId, bytes: usize, now: Instant) -> Instant {
        assert!(bytes > 0 && bytes <= PACE_SLICE, "invalid pacing slice");
        let mut state = self.schedule.lock().expect("poisoned rate scheduling mutex");
        let peer_next = state.peers_next.get(&peer).copied().unwrap_or(now);
        let slot = now.max(state.global_next).max(peer_next);
        // Rate is bytes per second; one scheduled 8 KiB fragment can be sent
        // immediately, while the next one must wait its assigned duration.
        // Integer nanosecond math gives deterministic monotonic scheduling.
        let ns_global = (bytes as u128 * 1_000_000_000u128)
            .div_ceil(self.global_bps as u128).max(1).min(u64::MAX as u128);
        let ns_peer = (bytes as u128 * 1_000_000_000u128)
            .div_ceil(self.per_peer_bps as u128).max(1).min(u64::MAX as u128);
        state.global_next = slot + Duration::from_nanos(ns_global as u64);
        state.peers_next.insert(peer, slot + Duration::from_nanos(ns_peer as u64));
        slot
    }

    /// Reserve outgoing application bytes across all streams and sleep until eligible.
    pub async fn wait(&self, peer: EndpointId, bytes: usize) {
        let slot = self.reserve_at(peer, bytes, Instant::now());
        sleep_until(slot).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    #[test]
    fn invalid_rates_are_refused() {
        assert!(RateGate::new(0, MIN_BYTES_PER_SEC).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC, 0).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC, MIN_BYTES_PER_SEC - 1).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC, MIN_BYTES_PER_SEC * 2).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC * 2, MIN_BYTES_PER_SEC).is_ok());
    }

    #[test]
    fn global_budget_is_shared_even_if_peer_ids_differ() {
        let gate = RateGate::new(16 * 1024, 16 * 1024).unwrap();
        let a = SecretKey::generate().public();
        let b = SecretKey::generate().public();
        let now = Instant::now();
        let t1 = gate.reserve_at(a, PACE_SLICE, now);
        let t2 = gate.reserve_at(b, PACE_SLICE, now);
        let t3 = gate.reserve_at(a, PACE_SLICE, now);
        assert!(t2.duration_since(t1) >= Duration::from_millis(500));
        assert!(t3.duration_since(t1) >= Duration::from_secs(1));
    }

    #[test]
    fn per_peer_budget_caps_a_single_recipient_below_global_capacity() {
        let gate = RateGate::new(128 * 1024, 16 * 1024).unwrap();
        let a = SecretKey::generate().public();
        let b = SecretKey::generate().public();
        let now = Instant::now();
        let t1 = gate.reserve_at(a, PACE_SLICE, now);
        let t2 = gate.reserve_at(a, PACE_SLICE, now);
        let t3 = gate.reserve_at(b, PACE_SLICE, now);
        assert!(t2.duration_since(t1) >= Duration::from_millis(500));
        assert!(t3 >= t2, "another peer does not escape the global reservation order");
    }

    #[tokio::test]
    async fn real_clock_blocks_second_fragment() {
        let gate = RateGate::new(16 * 1024, 16 * 1024).unwrap();
        let peer = SecretKey::generate().public();
        let now = Instant::now();
        gate.wait(peer, PACE_SLICE).await;
        gate.wait(peer, PACE_SLICE).await;
        assert!(now.elapsed() >= Duration::from_millis(475));
    }
}
