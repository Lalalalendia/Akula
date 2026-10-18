//! Admission budget at Akula's Roha QUIC-connection handoff.
//!
//! This is an APPLICATION-LEVEL limit: Roha has already received UDP packets
//! before Endpoint::accept yields an Incoming. It does not protect the socket,
//! QUIC parser, kernel buffers, TLS internals or network bandwidth. Real
//! pre-QUIC throttling is separate work in our Roha/noq forks.
//!
//! The shared limits are implemented by our pinned WOW research crate.
//! No remotely claimed DeviceId is consulted before QUIC authenticates a peer.

use std::sync::{Arc, Mutex};
use std::time::Instant;

pub use akula_wire_lab::ingress_guard_w5::{AdmissionError, AdmissionStats};
use akula_wire_lab::ingress_guard_w5::{IngressGuard, Lane, Ticket};

struct State {
    clock: Instant,
    guard: Mutex<IngressGuard>,
}

/// Cloneable admission state shared across all calls to accept_one.
#[derive(Clone)]
pub struct HandshakeGate {
    state: Arc<State>,
}

/// RAII guard: even an aborted future or failed QUIC handshake releases a slot.
/// Attempt counters deliberately remain charged until the monotonic tick ends.
pub struct PendingHandshake {
    state: Arc<State>,
    ticket: Ticket,
}

impl HandshakeGate {
    pub fn new() -> Self {
        Self {
            state: Arc::new(State {
                clock: Instant::now(),
                guard: Mutex::new(IngressGuard::new()),
            }),
        }
    }

    /// Only LAN handoffs are implemented in NET-DIRECT-13a.
    /// No claim of transport-level or cross-process rate limiting.
    pub fn reserve_lan(&self) -> Result<PendingHandshake, AdmissionError> {
        let mut guard = self
            .state
            .guard
            .lock()
            .map_err(|_| AdmissionError::AtCapacity)?;
        guard.advance_tick(self.state.clock.elapsed().as_secs())?;
        // The remote identity is not yet authenticated. Never use a claimed
        // ID to create a separate quota bucket or bypass the shared limit.
        let ticket = guard.begin(Lane::LocalNetwork, [0u8; 16])?;
        Ok(PendingHandshake {
            state: self.state.clone(),
            ticket,
        })
    }

    pub fn stats(&self) -> Result<AdmissionStats, AdmissionError> {
        let guard = self
            .state
            .guard
            .lock()
            .map_err(|_| AdmissionError::AtCapacity)?;
        Ok(guard.stats())
    }
}

impl Default for HandshakeGate {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HandshakeGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandshakeGate").finish_non_exhaustive()
    }
}

impl Drop for PendingHandshake {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.state.guard.lock() {
            let _ = guard.cancel(self.ticket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_and_cancellation_do_not_leave_open_slots() {
        let gate = HandshakeGate::new();
        let mut held = Vec::new();
        for _ in 0..8 {
            held.push(gate.reserve_lan().expect("within LAN capacity"));
        }
        assert!(matches!(gate.reserve_lan(), Err(AdmissionError::AtCapacity)));
        assert_eq!(gate.stats().unwrap().active, 8);
        drop(held);
        assert_eq!(gate.stats().unwrap().active, 0);

        // Slot release must not reset the per-second attempt limit.
        for _ in 0..3 {
            drop(gate.reserve_lan().unwrap());
        }
        assert!(matches!(gate.reserve_lan(), Err(AdmissionError::RateLimited)));
    }

    #[test]
    fn all_clones_use_one_shared_budget() {
        let gate = HandshakeGate::new();
        let other = gate.clone();
        for _ in 0..12 {
            drop(other.reserve_lan().unwrap());
        }
        assert!(matches!(gate.reserve_lan(), Err(AdmissionError::RateLimited)));
        assert_eq!(gate.stats().unwrap().attempts_this_tick, 12);
    }
}
