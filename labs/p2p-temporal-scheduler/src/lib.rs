//! Synthetic, offline-only experiment for scheduling verified public P2P chunks.
//! This crate is deliberately outside the shipping Akula workspace.
//! It has no network, filesystem, authorization or S3 APIs.
pub mod duration;
pub mod oracle;

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

const SLOT_COST: u64 = 1_000;

#[derive(Clone, Debug)]
pub struct Peer {
    /// Whether the device is online in each discrete time slot.
    pub online: Vec<bool>,
    /// Initially verified, immutable public chunks. No private Vault files.
    pub cached: Vec<bool>,
    pub upload_slots: usize,
    pub download_slots: usize,
    /// Only a consenting peer may upload.
    pub share_enabled: bool,
    /// Whether this peer explicitly accepts unsolicited public-chunk prefetching.
    /// A user-requested Demand authorizes receiving only that demanded chunk.
    pub prefetch_opt_in: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Demand {
    pub receiver: usize,
    pub chunk: usize,
    /// Last slot in which the transfer may start and finish at its end.
    pub deadline: usize,
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub peers: Vec<Peer>,
    /// Undirected peer connection eligibility; not a claim of authorization.
    pub links: Vec<(usize, usize)>,
    pub chunks: usize,
    pub horizon: usize,
    /// Shared maximum number of S3 chunk transfers per slot.
    pub s3_upload_slots: usize,
    pub demands: Vec<Demand>,
}

impl Scenario {
    pub fn validate(&self) -> Result<(), String> {
        if self.peers.is_empty() || self.horizon == 0 || self.chunks == 0 {
            return Err("peers, horizon and chunks must be nonempty".into());
        }
        for (id, peer) in self.peers.iter().enumerate() {
            if peer.online.len() != self.horizon || peer.cached.len() != self.chunks {
                return Err(format!("peer {id} has inconsistent online/cache lengths"));
            }
        }
        for &(a, b) in &self.links {
            if a >= self.peers.len() || b >= self.peers.len() || a == b {
                return Err("invalid peer link".into());
            }
        }
        let mut unique = HashSet::new();
        for demand in &self.demands {
            if demand.receiver >= self.peers.len()
                || demand.chunk >= self.chunks
                || demand.deadline >= self.horizon
            {
                return Err("invalid demand".into());
            }
            if !unique.insert((demand.receiver, demand.chunk)) {
                return Err("duplicate receiver/chunk demand".into());
            }
        }
        Ok(())
    }

    fn adjacency(&self) -> Vec<Vec<usize>> {
        let mut adj = vec![Vec::new(); self.peers.len()];
        for &(a, b) in &self.links {
            adj[a].push(b);
            adj[b].push(a);
        }
        for row in &mut adj {
            row.sort_unstable();
            row.dedup();
        }
        adj
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Source {
    S3,
    Peer(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transfer {
    pub slot: usize,
    pub from: Source,
    pub to: usize,
    pub chunk: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Strategy {
    S3Only,
    GreedyOnDemand,
    TemporalRoute,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub completed: usize,
    pub total: usize,
    /// Each transfer represents one equal-size verified public chunk.
    pub s3_chunks: usize,
    pub peer_chunks: usize,
    /// Moment immediately after the last completed demand's receiving slot.
    pub last_completion: Option<usize>,
    pub transfers: Vec<Transfer>,
}

impl Outcome {
    /// Independently replay all transfers, including limits and same-slot causality.
    pub fn verify(&self, s: &Scenario) -> Result<(), String> {
        s.validate()?;
        let adj = s.adjacency();
        let mut working = Working::new(s);
        let mut sorted = self.transfers.clone();
        sorted.sort_by_key(|x| (x.slot, x.to, x.chunk));
        for tr in sorted {
            if let Source::Peer(id) = tr.from {
                if id >= s.peers.len()
                    || !working.ready[id][tr.chunk.min(s.chunks - 1)]
                        .is_some_and(|ready| ready <= tr.slot)
                {
                    return Err("peer tried to upload before owning a verified chunk".into());
                }
            }
            if !working.can_transfer(s, &adj, tr) {
                return Err(format!("invalid or duplicate transfer: {tr:?}"));
            }
            working.commit(tr);
        }
        let checked = working.outcome(s);
        if (
            checked.completed,
            checked.total,
            checked.s3_chunks,
            checked.peer_chunks,
        ) != (self.completed, self.total, self.s3_chunks, self.peer_chunks)
            || checked.last_completion != self.last_completion
        {
            return Err("reported outcome does not match replay".into());
        }
        Ok(())
    }
}

struct Working {
    /// Earliest point at which a chunk can be uploaded by that peer.
    ready: Vec<Vec<Option<usize>>>,
    uploads: Vec<Vec<usize>>,
    downloads: Vec<Vec<usize>>,
    s3_used: Vec<usize>,
    transfers: Vec<Transfer>,
}

impl Working {
    fn new(s: &Scenario) -> Self {
        Self {
            ready: s
                .peers
                .iter()
                .map(|p| p.cached.iter().map(|&has| has.then_some(0)).collect())
                .collect(),
            uploads: vec![vec![0; s.horizon]; s.peers.len()],
            downloads: vec![vec![0; s.horizon]; s.peers.len()],
            s3_used: vec![0; s.horizon],
            transfers: Vec::new(),
        }
    }

    fn can_transfer(&self, s: &Scenario, adj: &[Vec<usize>], tr: Transfer) -> bool {
        if tr.slot >= s.horizon
            || tr.to >= s.peers.len()
            || tr.chunk >= s.chunks
            || !s.peers[tr.to].online[tr.slot]
            || self.downloads[tr.to][tr.slot] >= s.peers[tr.to].download_slots
            || self.ready[tr.to][tr.chunk].is_some()
            || (!s.peers[tr.to].prefetch_opt_in
                && !s
                    .demands
                    .iter()
                    .any(|d| d.receiver == tr.to && d.chunk == tr.chunk))
        {
            return false;
        }
        match tr.from {
            Source::S3 => self.s3_used[tr.slot] < s.s3_upload_slots,
            Source::Peer(src) => {
                src < s.peers.len()
                    && s.peers[src].share_enabled
                    && s.peers[src].online[tr.slot]
                    && self.uploads[src][tr.slot] < s.peers[src].upload_slots
                    && adj[src].binary_search(&tr.to).is_ok()
            }
        }
    }

    fn commit(&mut self, tr: Transfer) {
        match tr.from {
            Source::S3 => self.s3_used[tr.slot] += 1,
            Source::Peer(src) => self.uploads[src][tr.slot] += 1,
        }
        self.downloads[tr.to][tr.slot] += 1;
        self.ready[tr.to][tr.chunk] = Some(tr.slot + 1);
        self.transfers.push(tr);
    }

    fn outcome(self, s: &Scenario) -> Outcome {
        let mut completed = 0;
        let mut latest = None;
        for d in &s.demands {
            if let Some(ready) = self.ready[d.receiver][d.chunk] {
                if ready <= d.deadline + 1 {
                    completed += 1;
                    latest = Some(latest.map_or(ready, |v: usize| v.max(ready)));
                }
            }
        }
        let s3_chunks = self
            .transfers
            .iter()
            .filter(|t| t.from == Source::S3)
            .count();
        let peer_chunks = self.transfers.len() - s3_chunks;
        Outcome {
            completed,
            total: s.demands.len(),
            s3_chunks,
            peer_chunks,
            last_completion: latest,
            transfers: self.transfers,
        }
    }

    /// A one-demand earliest-arrival route through a time-expanded graph.
    /// Waiting keeps the chunk on disk even if the peer is temporarily offline.
    fn route(&self, s: &Scenario, adj: &[Vec<usize>], d: Demand) -> Option<Vec<Transfer>> {
        let count = s.peers.len() + 1;
        let s3 = s.peers.len();
        let state_count = (s.horizon + 1) * count;
        let id = |t: usize, node: usize| t * count + node;
        let mut distance = vec![u64::MAX; state_count];
        let mut previous: Vec<Option<(usize, Option<Transfer>)>> = vec![None; state_count];
        let mut heap = BinaryHeap::new();

        // Existing verified copies are already scheduled or present at t=0.
        for node in 0..s.peers.len() {
            if let Some(ready_at) = self.ready[node][d.chunk] {
                if ready_at <= d.deadline + 1 {
                    let index = id(ready_at, node);
                    let cost = ready_at as u64 * SLOT_COST;
                    distance[index] = cost;
                    heap.push(Reverse((cost, ready_at, node)));
                }
            }
        }
        distance[id(0, s3)] = 0;
        heap.push(Reverse((0, 0, s3)));

        while let Some(Reverse((cost, t, node))) = heap.pop() {
            let current = id(t, node);
            if cost != distance[current] {
                continue;
            }
            if node == d.receiver && t <= d.deadline + 1 {
                let mut path = Vec::new();
                let mut at = current;
                while let Some((prev, edge)) = previous[at] {
                    if let Some(edge) = edge {
                        path.push(edge);
                    }
                    at = prev;
                }
                path.reverse();
                return Some(path);
            }
            if t > d.deadline {
                continue;
            }
            // Storage/waiting does not require an online device.
            let wait = id(t + 1, node);
            if cost + SLOT_COST < distance[wait] {
                distance[wait] = cost + SLOT_COST;
                previous[wait] = Some((current, None));
                heap.push(Reverse((distance[wait], t + 1, node)));
            }
            let from = if node == s3 {
                Source::S3
            } else {
                Source::Peer(node)
            };
            let mut consider = |target: usize| {
                let tr = Transfer {
                    slot: t,
                    from,
                    to: target,
                    chunk: d.chunk,
                };
                if !self.can_transfer(s, adj, tr) {
                    return;
                }
                let next = id(t + 1, target);
                let price = cost + SLOT_COST + u64::from(from == Source::S3);
                if price < distance[next] {
                    distance[next] = price;
                    previous[next] = Some((current, Some(tr)));
                    heap.push(Reverse((price, t + 1, target)));
                }
            };
            if node == s3 {
                for target in 0..s.peers.len() {
                    consider(target);
                }
            } else {
                for &target in &adj[node] {
                    consider(target);
                }
            }
        }
        None
    }
}

/// Schedule with no real network I/O. Peer transfers take one full slot;
/// a freshly received chunk may be uploaded only in a later slot.
pub fn schedule(s: &Scenario, strategy: Strategy) -> Result<Outcome, String> {
    s.validate()?;
    let adj = s.adjacency();
    let mut working = Working::new(s);
    let mut demands = s.demands.clone();
    demands.sort_by_key(|d| (d.deadline, d.receiver, d.chunk));

    match strategy {
        Strategy::TemporalRoute => {
            for d in demands {
                if working.ready[d.receiver][d.chunk].is_some_and(|at| at <= d.deadline + 1) {
                    continue;
                }
                if let Some(route) = working.route(s, &adj, d) {
                    for tr in route {
                        // Each leg is reserved before planning the next demand.
                        debug_assert!(working.can_transfer(s, &adj, tr));
                        working.commit(tr);
                    }
                }
            }
        }
        Strategy::S3Only | Strategy::GreedyOnDemand => {
            for t in 0..s.horizon {
                for &d in &demands {
                    if t > d.deadline || working.ready[d.receiver][d.chunk].is_some() {
                        continue;
                    }
                    let from_peer = if strategy == Strategy::GreedyOnDemand {
                        adj[d.receiver].iter().copied().find(|&peer| {
                            working.ready[peer][d.chunk].is_some_and(|at| at <= t)
                                && working.can_transfer(
                                    s,
                                    &adj,
                                    Transfer {
                                        slot: t,
                                        from: Source::Peer(peer),
                                        to: d.receiver,
                                        chunk: d.chunk,
                                    },
                                )
                        })
                    } else {
                        None
                    };
                    let tr = Transfer {
                        slot: t,
                        from: from_peer.map_or(Source::S3, Source::Peer),
                        to: d.receiver,
                        chunk: d.chunk,
                    };
                    if working.can_transfer(s, &adj, tr) {
                        working.commit(tr);
                    }
                }
            }
        }
    }

    let outcome = working.outcome(s);
    outcome.verify(s)?;
    Ok(outcome)
}

/// A deterministic demonstration of non-overlapping contact windows.
/// A is online only at slot 0, B/D only at slot 2, bridge R at 0 and 2.
/// R can distribute to both late receivers after caching at slot 0.
pub fn bridge_demo() -> Scenario {
    Scenario {
        chunks: 1,
        horizon: 4,
        s3_upload_slots: 1,
        peers: vec![
            Peer {
                online: vec![true, false, false, false],
                cached: vec![true],
                upload_slots: 1,
                download_slots: 1,
                share_enabled: true,
                prefetch_opt_in: false,
            },
            Peer {
                online: vec![true, false, true, false],
                cached: vec![false],
                upload_slots: 2,
                download_slots: 1,
                share_enabled: true,
                prefetch_opt_in: true,
            },
            Peer {
                online: vec![false, false, true, false],
                cached: vec![false],
                upload_slots: 0,
                download_slots: 1,
                share_enabled: false,
                prefetch_opt_in: false,
            },
            Peer {
                online: vec![false, false, true, false],
                cached: vec![false],
                upload_slots: 0,
                download_slots: 1,
                share_enabled: false,
                prefetch_opt_in: false,
            },
        ],
        links: vec![(0, 1), (1, 2), (1, 3)],
        demands: vec![
            Demand {
                receiver: 2,
                chunk: 0,
                deadline: 2,
            },
            Demand {
                receiver: 3,
                chunk: 0,
                deadline: 2,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_three(s: &Scenario) -> Vec<Outcome> {
        [
            Strategy::S3Only,
            Strategy::GreedyOnDemand,
            Strategy::TemporalRoute,
        ]
        .into_iter()
        .map(|kind| schedule(s, kind).unwrap())
        .collect()
    }

    #[test]
    fn bridge_reduces_s3_and_serves_both_receivers() {
        let r = all_three(&bridge_demo());
        assert_eq!(r[0].completed, 1);
        assert_eq!(r[1].completed, 1);
        assert_eq!(r[2].completed, 2);
        assert_eq!(r[2].s3_chunks, 0);
        assert_eq!(r[2].peer_chunks, 3);
        assert!(r[2].transfers.contains(&Transfer {
            slot: 0,
            from: Source::Peer(0),
            to: 1,
            chunk: 0,
        }));
    }

    #[test]
    fn consent_is_not_inferred_from_a_cached_chunk() {
        let mut s = bridge_demo();
        s.peers[0].share_enabled = false;
        s.s3_upload_slots = 0;
        assert_eq!(schedule(&s, Strategy::TemporalRoute).unwrap().completed, 0);
    }

    #[test]
    fn a_bridge_must_opt_in_to_unsolicited_prefetch() {
        let mut s = bridge_demo();
        s.peers[1].prefetch_opt_in = false;
        s.s3_upload_slots = 0;
        assert_eq!(schedule(&s, Strategy::TemporalRoute).unwrap().completed, 0);
    }

    #[test]
    fn an_unconnected_peer_cannot_deliver_a_chunk() {
        let mut s = bridge_demo();
        s.links.clear();
        s.s3_upload_slots = 0;
        assert_eq!(schedule(&s, Strategy::TemporalRoute).unwrap().completed, 0);
    }

    #[test]
    fn a_newly_received_chunk_cannot_be_forwarded_in_the_same_slot() {
        let mut s = bridge_demo();
        s.horizon = 1;
        for p in &mut s.peers {
            p.online = vec![true];
        }
        s.demands.truncate(1);
        s.demands[0].deadline = 0;
        s.s3_upload_slots = 0;
        assert_eq!(schedule(&s, Strategy::TemporalRoute).unwrap().completed, 0);
    }

    #[test]
    fn expired_deadlines_are_not_counted() {
        let mut s = bridge_demo();
        s.demands.iter_mut().for_each(|d| d.deadline = 1);
        s.s3_upload_slots = 0;
        assert_eq!(schedule(&s, Strategy::TemporalRoute).unwrap().completed, 0);
    }

    #[test]
    fn s3_is_a_fallback_when_no_peer_can_help() {
        let mut s = bridge_demo();
        s.peers[0].share_enabled = false;
        s.demands.truncate(1);
        let p = schedule(&s, Strategy::TemporalRoute).unwrap();
        assert_eq!(p.completed, 1);
        assert_eq!(p.s3_chunks, 1);
    }

    #[test]
    fn peer_upload_capacity_is_enforced() {
        let mut s = bridge_demo();
        s.peers[1].upload_slots = 1;
        s.s3_upload_slots = 0;
        let p = schedule(&s, Strategy::TemporalRoute).unwrap();
        assert_eq!(p.completed, 1);
        p.verify(&s).unwrap();
    }

    #[test]
    fn malformed_and_duplicate_demands_are_rejected() {
        let mut s = bridge_demo();
        s.demands.push(s.demands[0]);
        assert!(s.validate().is_err());
    }

    #[test]
    fn tampered_schedule_fails_independent_replay() {
        let s = bridge_demo();
        let mut p = schedule(&s, Strategy::TemporalRoute).unwrap();
        p.transfers[0].slot = 3;
        assert!(p.verify(&s).is_err());
    }

    #[test]
    fn strategy_results_are_deterministic() {
        let s = bridge_demo();
        let a = schedule(&s, Strategy::TemporalRoute).unwrap();
        let b = schedule(&s, Strategy::TemporalRoute).unwrap();
        assert_eq!(a.transfers, b.transfers);
    }
}
