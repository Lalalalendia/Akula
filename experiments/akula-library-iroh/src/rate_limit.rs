//! Fair, bounded in-process application-byte pacing for the isolated library lab.
//!
//! A *single* active identity cannot reserve the entire future time line: an
//! asynchronous arbiter grants at most one 8-KiB slice at a time, taking turns
//! among identities that currently have a pending request.
//!
//! This is not a full-IP/UDP-wire budget, a persistent quota, or Sybil protection.

use anyhow::{anyhow, bail, ensure, Context, Result};
use iroh::EndpointId;
use std::{collections::{HashMap, VecDeque}, sync::Mutex};
use tokio::{
    sync::{mpsc, oneshot},
    time::{sleep_until, Duration, Instant},
};

pub const PACE_SLICE: usize = 8 * 1024;
pub const MIN_BYTES_PER_SEC: u64 = 16 * 1024;
const CHANNEL_CAPACITY: usize = 512;
const MAX_PENDING: usize = 512;
const MAX_PER_PEER_PENDING: usize = 4;

struct Request {
    peer: EndpointId,
    bytes: usize,
    granted: oneshot::Sender<Result<()>>,
}

struct Scheduler {
    global_bps: u64,
    peer_bps: u64,
    global_next: Instant,
    peer_next: HashMap<EndpointId, Instant>,
    pending: HashMap<EndpointId, VecDeque<Request>>,
    active: VecDeque<EndpointId>,
    queued: usize,
    grants: usize,
}

impl Scheduler {
    fn new(global_bps: u64, peer_bps: u64, now: Instant) -> Self {
        Self {
            global_bps, peer_bps, global_next: now,
            peer_next: HashMap::new(), pending: HashMap::new(),
            active: VecDeque::new(), queued: 0, grants: 0,
        }
    }

    fn enqueue(&mut self, req: Request) {
        if req.granted.is_closed() { return; }
        if self.queued >= MAX_PENDING {
            let _ = req.granted.send(Err(anyhow!("rate queue is full")));
            return;
        }
        let peer = req.peer;
        let per_peer_len = self.pending.get(&peer).map_or(0, VecDeque::len);
        if per_peer_len >= MAX_PER_PEER_PENDING {
            let _ = req.granted.send(Err(anyhow!("rate queue for peer is full")));
            return;
        }
        if per_peer_len == 0 { self.active.push_back(peer); }
        self.pending.entry(peer).or_default().push_back(req);
        self.queued += 1;
    }

    fn prune_canceled(&mut self, now: Instant) {
        for queue in self.pending.values_mut() {
            let before = queue.len();
            queue.retain(|r| !r.granted.is_closed());
            self.queued -= before - queue.len();
        }
        self.pending.retain(|_, q| !q.is_empty());
        self.active.retain(|peer| self.pending.contains_key(peer));
        // Reclaim identities that no longer have queued work AND no pacing debt.
        if self.grants % 256 == 0 {
            self.peer_next.retain(|peer, until| *until > now || self.pending.contains_key(peer));
        }
    }

    /// Earliest eligible peer. Ties use the FIFO active peer order; after every
    /// grant the chosen peer moves to the end. Per-peer slowdowns do not stall
    /// unrelated identities.
    fn candidate(&self, now: Instant) -> Option<(EndpointId, Instant)> {
        self.active.iter().copied().filter_map(|peer| {
            self.pending.get(&peer).and_then(|q| q.front()).map(|_| {
                let next = self.peer_next.get(&peer).copied().unwrap_or(now);
                (peer, now.max(self.global_next).max(next))
            })
        }).min_by_key(|(_, when)| *when)
    }

    fn duration(bytes: usize, bps: u64) -> Duration {
        let ns = (bytes as u128 * 1_000_000_000u128)
            .div_ceil(bps as u128).max(1).min(u64::MAX as u128);
        Duration::from_nanos(ns as u64)
    }

    /// Single scheduling decision, never pre-allocating many future slices.
    /// The caller must ensure the selected time has arrived.
    fn grant(&mut self, peer: EndpointId, when: Instant) -> bool {
        let Some(queue) = self.pending.get_mut(&peer) else { return false };
        let Some(req) = queue.pop_front() else { return false };
        self.queued -= 1;
        self.active.retain(|id| *id != peer);
        if queue.is_empty() {
            self.pending.remove(&peer);
        } else {
            self.active.push_back(peer);
        }
        if req.granted.is_closed() { return false; }
        let when = when.max(Instant::now());
        self.global_next = when + Self::duration(req.bytes, self.global_bps);
        self.peer_next.insert(peer, when + Self::duration(req.bytes, self.peer_bps));
        self.grants += 1;
        // Once a slot is granted, no refund for a task that cancels after the
        // decision. This bounds rather than understates scheduled data.
        let _ = req.granted.send(Ok(()));
        true
    }
}

pub struct RateGate {
    sender: mpsc::Sender<Request>,
    receiver: Mutex<Option<mpsc::Receiver<Request>>>,
    global_bps: u64,
    peer_bps: u64,
}

impl RateGate {
    pub fn new(global_bps: u64, peer_bps: u64) -> Result<Self> {
        ensure!(global_bps >= MIN_BYTES_PER_SEC, "global rate too low (min 16384 bytes/s)");
        ensure!(peer_bps >= MIN_BYTES_PER_SEC, "peer rate too low (min 16384 bytes/s)");
        ensure!(peer_bps <= global_bps, "peer rate must not exceed global rate");
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        Ok(Self { sender, receiver: Mutex::new(Some(receiver)), global_bps, peer_bps })
    }

    fn start_if_needed(&self) {
        let receiver = self.receiver.lock().expect("rate gate receiver poisoned").take();
        if let Some(receiver) = receiver {
            tokio::spawn(Self::serve(receiver, self.global_bps, self.peer_bps));
        }
    }

    async fn serve(mut rx: mpsc::Receiver<Request>, global_bps: u64, peer_bps: u64) {
        let mut scheduler = Scheduler::new(global_bps, peer_bps, Instant::now());
        let mut open = true;
        loop {
            // Drain competing requests first, rather than letting one eager
            // producer monopolize a future timetable before peers arrive.
            while let Ok(req) = rx.try_recv() { scheduler.enqueue(req); }
            scheduler.prune_canceled(Instant::now());

            if let Some((peer, due)) = scheduler.candidate(Instant::now()) {
                if due <= Instant::now() {
                    scheduler.grant(peer, due);
                    continue;
                }
                // A new peer may have an earlier eligible time than a peer
                // waiting on its individual rate. Re-evaluate immediately.
                tokio::select! {
                    new = rx.recv(), if open => match new {
                        Some(req) => scheduler.enqueue(req),
                        None => open = false,
                    },
                    _ = sleep_until(due) => {},
                }
            } else if open {
                match rx.recv().await {
                    Some(req) => scheduler.enqueue(req),
                    None => open = false,
                }
            } else {
                break;
            }
        }
    }

    /// Await a fair turn for one application-byte slice.
    /// The semaphore on active QUIC connections is separate from this queue.
    pub async fn wait(&self, peer: EndpointId, bytes: usize) -> Result<()> {
        if bytes == 0 || bytes > PACE_SLICE {
            bail!("rate pacing slice must be in 1..={PACE_SLICE}");
        }
        self.start_if_needed();
        let (granted, rx) = oneshot::channel();
        self.sender.send(Request { peer, bytes, granted }).await
            .context("rate arbiter channel unavailable")?;
        rx.await.context("rate arbiter ended before responding")?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;
    use std::collections::HashSet;

    fn sample_peer() -> EndpointId { SecretKey::generate().public() }

    #[test]
    fn rejects_invalid_limits() {
        assert!(RateGate::new(0, MIN_BYTES_PER_SEC).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC, 0).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC, MIN_BYTES_PER_SEC*2).is_err());
        assert!(RateGate::new(MIN_BYTES_PER_SEC*2, MIN_BYTES_PER_SEC).is_ok());
    }

    /// Virtual-time saturation: each group of N global slots has all N peers,
    /// despite the first peer being able to enqueue all its own work first.
    fn virtual_peer_fairness(n: usize) {
        let peers: Vec<_> = (0..n).map(|_| sample_peer()).collect();
        let mut scheduler = Scheduler::new(1024*1024, 1024*1024, Instant::now());
        let mut keep_receivers = Vec::new();
        for peer in &peers {
            for _ in 0..4 {
                let (tx, rx) = oneshot::channel();
                scheduler.enqueue(Request { peer: *peer, bytes: PACE_SLICE, granted: tx });
                keep_receivers.push(rx);
            }
        }
        assert_eq!(scheduler.queued, 4*n);
        let mut now = Instant::now();
        for round in 0..4 {
            let mut seen = HashSet::new();
            for _ in 0..n {
                let (peer, when) = scheduler.candidate(now).unwrap();
                seen.insert(peer);
                now = when.max(now);
                assert!(scheduler.grant(peer, now));
            }
            assert_eq!(seen.len(), n, "peer starved in round {round} of {n}");
        }
        assert_eq!(scheduler.queued, 0);
        assert_eq!(keep_receivers.len(), 4*n);
    }

    #[test]
    fn ten_peers_receive_one_slot_per_round() { virtual_peer_fairness(10); }

    #[test]
    fn hundred_peers_receive_one_slot_per_round() { virtual_peer_fairness(100); }

    #[test]
    fn single_hog_cannot_fill_future_queue() {
        let a = sample_peer();
        let mut scheduler = Scheduler::new(1024*1024, 1024*1024, Instant::now());
        let mut rx = Vec::new();
        for _ in 0..100 {
            let (tx, r) = oneshot::channel();
            scheduler.enqueue(Request { peer: a, bytes: PACE_SLICE, granted: tx });
            rx.push(r);
        }
        assert_eq!(scheduler.queued, MAX_PER_PEER_PENDING);
        assert_eq!(rx.len(), 100);
    }

    #[test]
    fn slow_peer_does_not_block_other_peers() {
        let a = sample_peer();
        let b = sample_peer();
        let mut scheduler = Scheduler::new(1024*1024, 16*1024, Instant::now());
        let (ta1, ra1) = oneshot::channel();
        let (ta2, ra2) = oneshot::channel();
        let (tb, rb) = oneshot::channel();
        let mut hold = vec![ra1, ra2, rb];
        scheduler.enqueue(Request{peer: a, bytes: PACE_SLICE, granted: ta1});
        scheduler.enqueue(Request{peer: a, bytes: PACE_SLICE, granted: ta2});
        scheduler.enqueue(Request{peer: b, bytes: PACE_SLICE, granted: tb});
        let now = Instant::now();
        let (first, t0) = scheduler.candidate(now).unwrap();
        assert_eq!(first, a);
        assert!(scheduler.grant(first, t0));
        let (second, t1) = scheduler.candidate(t0).unwrap();
        assert_eq!(second, b, "slow peer ahead cannot block other ready peer");
        assert!(t1.duration_since(t0) < Duration::from_millis(30));
        assert!(scheduler.grant(second, t1));
        let (_, t2) = scheduler.candidate(t1).unwrap();
        assert!(t2.duration_since(t0) >= Duration::from_millis(490));
        assert_eq!(hold.len(),3);
        hold.clear();
    }

    #[test]
    fn canceled_requests_free_queue_capacity() {
        let peer = sample_peer();
        let mut scheduler = Scheduler::new(1024*1024, 1024*1024, Instant::now());
        let (tx, rx) = oneshot::channel();
        scheduler.enqueue(Request { peer, bytes: PACE_SLICE, granted: tx });
        assert_eq!(scheduler.queued, 1);
        drop(rx);
        scheduler.prune_canceled(Instant::now());
        assert_eq!(scheduler.queued, 0);
        assert!(scheduler.candidate(Instant::now()).is_none());
    }

    #[tokio::test]
    async fn actual_monotonic_delay_limits_second_slice() -> Result<()> {
        let gate = RateGate::new(16*1024,16*1024)?;
        let peer = sample_peer();
        let start=Instant::now();
        gate.wait(peer,PACE_SLICE).await?;
        gate.wait(peer,PACE_SLICE).await?;
        assert!(start.elapsed() >= Duration::from_millis(470));
        Ok(())
    }

    #[tokio::test]
    async fn canceling_a_waiter_does_not_halt_other_traffic() -> Result<()> {
        let gate = std::sync::Arc::new(RateGate::new(16*1024,16*1024)?);
        let peer_a=sample_peer();
        let peer_b=sample_peer();
        gate.wait(peer_a,PACE_SLICE).await?;
        let cancel = tokio::spawn({
            let gate = gate.clone();
            async move { let _ = gate.wait(peer_a, PACE_SLICE).await; }
        });
        cancel.abort();
        tokio::time::timeout(Duration::from_secs(3), gate.wait(peer_b, PACE_SLICE)).await??;
        Ok(())
    }
}
