//! Offline-only synthetic verified-segment recovery and slot-by-slot replanning.
//!
//! Each MiB is assumed independently verifiable against an immutable public
//! content commitment. This module performs NO cryptography, I/O, or grants.
//! A partially verified chunk can be resumed from another fully verified
//! source, but cannot be uploaded by the receiver until it is complete.

use std::collections::HashSet;

use crate::duration::{Network, Source};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Policy {
    S3Only,
    PeerFirst,
    ForecastDirect,
    AdaptiveRelay,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Segment {
    pub slot: usize,
    pub from: Source,
    pub to: usize,
    pub chunk: usize,
    /// Number of verified 1-MiB segments transferred in this online slot.
    pub mib: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Metrics {
    pub completed: usize,
    pub total: usize,
    pub s3_mib: usize,
    pub p2p_mib: usize,
    pub source_switches: usize,
    pub prefetched_mib: usize,
    pub incomplete_mib: usize,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub segments: Vec<Segment>,
    pub metrics: Metrics,
}

impl Plan {
    /// Replays all segment transfers against the observed trace and quotas.
    /// Checks source ownership at slot start, consent, capacities and metrics.
    pub fn verify(&self, network: &Network, actual: &[Vec<bool>]) -> Result<(), String> {
        let mut state = Working::new(network, actual)?;
        let mut ordered = self.segments.clone();
        ordered.sort_by_key(|x| (x.slot, x.to, x.chunk));
        if self.segments.windows(2).any(|pair| pair[0].slot > pair[1].slot) {
            return Err("segment slots must be nondecreasing".into());
        }
        let mut cursor = 0;
        for slot in 0..network.steps {
            let owners = state.full_snapshot();
            let mut budget = Budget::new(network);
            while cursor < ordered.len() && ordered[cursor].slot == slot {
                let entry = ordered[cursor];
                let amount = state
                    .possible(slot, entry.from, entry.to, entry.chunk, &owners, &budget)
                    .ok_or_else(|| format!("invalid recovered segment at slot {slot}"))?;
                if amount != entry.mib {
                    return Err(format!("segment length mismatch at slot {slot}"));
                }
                state.record(entry, &mut budget)?;
                cursor += 1;
            }
        }
        if cursor != ordered.len() {
            return Err("out-of-horizon segment transfer".into());
        }
        if state.summary() != self.metrics {
            return Err("reported recovery metrics do not match replay".into());
        }
        Ok(())
    }
}

struct Budget {
    uploads: Vec<usize>,
    downloads: Vec<usize>,
    edges: HashSet<usize>,
    s3_used: usize,
    receiver_chunk: HashSet<(usize, usize)>,
}

impl Budget {
    fn new(network: &Network) -> Self {
        Self {
            uploads: vec![0; network.nodes.len()],
            downloads: vec![0; network.nodes.len()],
            edges: HashSet::new(),
            s3_used: 0,
            receiver_chunk: HashSet::new(),
        }
    }
}

struct Working<'a> {
    network: &'a Network,
    actual: &'a [Vec<bool>],
    adjacency: Vec<Vec<(usize, usize)>>,
    received: Vec<Vec<usize>>,
    completed_at: Vec<Vec<Option<usize>>>,
    prior_source: Vec<Vec<Option<Source>>>,
    segments: Vec<Segment>,
    switches: usize,
    prefetch_mib: usize,
    s3_mib: usize,
    p2p_mib: usize,
}

impl<'a> Working<'a> {
    fn new(network: &'a Network, actual: &'a [Vec<bool>]) -> Result<Self, String> {
        network.validate()?;
        if actual.len() != network.nodes.len()
            || actual.iter().any(|x| x.len() != network.steps)
        {
            return Err("invalid observed online-trace dimensions".into());
        }
        let mut adjacency = vec![Vec::new(); network.nodes.len()];
        for (id, link) in network.links.iter().enumerate() {
            adjacency[link.a].push((link.b, id));
            adjacency[link.b].push((link.a, id));
        }
        for row in &mut adjacency {
            row.sort_unstable();
        }
        let received = network
            .nodes
            .iter()
            .map(|node| {
                node.cached
                    .iter()
                    .enumerate()
                    .map(|(chunk, &cached)| {
                        if cached { network.sizes_mib[chunk] } else { 0 }
                    })
                    .collect()
            })
            .collect();
        let completed_at = network
            .nodes
            .iter()
            .map(|node| node.cached.iter().map(|&cached| cached.then_some(0)).collect())
            .collect();
        Ok(Self {
            network,
            actual,
            adjacency,
            received,
            completed_at,
            prior_source: vec![vec![None; network.sizes_mib.len()]; network.nodes.len()],
            segments: Vec::new(),
            switches: 0,
            prefetch_mib: 0,
            s3_mib: 0,
            p2p_mib: 0,
        })
    }

    fn full_snapshot(&self) -> Vec<Vec<bool>> {
        self.received
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(chunk, &mib)| mib == self.network.sizes_mib[chunk])
                    .collect()
            })
            .collect()
    }

    fn link(&self, source: usize, to: usize) -> Option<(usize, usize)> {
        let &(_, index) = self.adjacency.get(source)?.iter().find(|(peer, _)| *peer == to)?;
        Some((index, self.network.links[index].rate_mib_per_slot))
    }

    fn rate(&self, from: Source, to: usize) -> Option<usize> {
        match from {
            Source::S3 => Some(self.network.s3_rate_mib_per_slot),
            Source::Peer(source) => Some(self.link(source, to)?.1),
        }
    }

    fn demand_deadline(&self, to: usize, chunk: usize) -> Option<usize> {
        self.network
            .requests
            .iter()
            .find(|r| r.receiver == to && r.chunk == chunk)
            .map(|r| r.last_slot)
    }

    fn possible(
        &self,
        slot: usize,
        from: Source,
        to: usize,
        chunk: usize,
        owners: &[Vec<bool>],
        budget: &Budget,
    ) -> Option<usize> {
        if to >= self.network.nodes.len()
            || chunk >= self.network.sizes_mib.len()
            || slot >= self.network.steps
            || !self.actual[to][slot]
            || budget.downloads[to] >= self.network.nodes[to].download_parallel
            || budget.receiver_chunk.contains(&(to, chunk))
            || self.received[to][chunk] >= self.network.sizes_mib[chunk]
        {
            return None;
        }
        if self.demand_deadline(to, chunk).is_none()
            && !self.network.nodes[to].prefetch_opt_in
        {
            return None;
        }
        let rate = match from {
            Source::S3 => {
                if budget.s3_used >= self.network.s3_parallel {
                    return None;
                }
                self.network.s3_rate_mib_per_slot
            }
            Source::Peer(source) => {
                if source == to
                    || source >= self.network.nodes.len()
                    || !owners[source][chunk]
                    || !self.actual[source][slot]
                    || !self.network.nodes[source].share_enabled
                    || budget.uploads[source] >= self.network.nodes[source].upload_parallel
                {
                    return None;
                }
                let (edge, rate) = self.link(source, to)?;
                if budget.edges.contains(&edge) {
                    return None;
                }
                rate
            }
        };
        Some(rate.min(self.network.sizes_mib[chunk] - self.received[to][chunk]))
    }

    fn record(&mut self, transfer: Segment, budget: &mut Budget) -> Result<(), String> {
        let Segment { slot, from, to, chunk, mib } = transfer;
        if mib == 0 || to >= self.received.len() || chunk >= self.network.sizes_mib.len() {
            return Err("zero-sized or invalid transfer".into());
        }
        if self.received[to][chunk] > 0
            && self.prior_source[to][chunk].is_some_and(|prior| prior != from)
        {
            self.switches += 1;
        }
        self.prior_source[to][chunk] = Some(from);
        self.received[to][chunk] += mib;
        if self.received[to][chunk] > self.network.sizes_mib[chunk] {
            return Err("overcomplete chunk".into());
        }
        if self.received[to][chunk] == self.network.sizes_mib[chunk] {
            self.completed_at[to][chunk] = Some(slot + 1);
        }
        budget.receiver_chunk.insert((to, chunk));
        budget.downloads[to] += 1;
        match from {
            Source::S3 => {
                budget.s3_used += 1;
                self.s3_mib += mib;
            }
            Source::Peer(source) => {
                budget.uploads[source] += 1;
                let (edge, _) = self.link(source, to).ok_or("invalid peer link")?;
                budget.edges.insert(edge);
                self.p2p_mib += mib;
            }
        }
        if self.demand_deadline(to, chunk).is_none() {
            self.prefetch_mib += mib;
        }
        self.segments.push(transfer);
        Ok(())
    }

    fn summary(&self) -> Metrics {
        let mut completed = 0;
        for request in &self.network.requests {
            if self.completed_at[request.receiver][request.chunk]
                .is_some_and(|slot| slot <= request.last_slot + 1)
            {
                completed += 1;
            }
        }
        let mut incomplete = 0;
        for (peer, row) in self.received.iter().enumerate() {
            for (chunk, &mib) in row.iter().enumerate() {
                if !self.network.nodes[peer].cached[chunk]
                    && mib < self.network.sizes_mib[chunk]
                {
                    incomplete += mib;
                }
            }
        }
        Metrics {
            completed,
            total: self.network.requests.len(),
            s3_mib: self.s3_mib,
            p2p_mib: self.p2p_mib,
            source_switches: self.switches,
            prefetched_mib: self.prefetch_mib,
            incomplete_mib: incomplete,
        }
    }

    fn predicted_finish(
        &self,
        slot: usize,
        from: Source,
        to: usize,
        chunk: usize,
        last_slot: usize,
    ) -> usize {
        let Some(rate) = self.rate(from, to) else {
            return usize::MAX;
        };
        let mut remaining = self.network.sizes_mib[chunk] - self.received[to][chunk];
        for t in slot..=last_slot {
            // The present slot is observed; never peek at a future actual state.
            let available = if t == slot {
                true
            } else {
                self.network.nodes[to].online[t]
                    && match from {
                        Source::S3 => true,
                        Source::Peer(src) => self.network.nodes[src].online[t],
                    }
            };
            if available {
                remaining = remaining.saturating_sub(rate);
                if remaining == 0 {
                    return t + 1;
                }
            }
        }
        usize::MAX
    }

    fn select(
        &self,
        policy: Policy,
        slot: usize,
        to: usize,
        chunk: usize,
        deadline: usize,
        owners: &[Vec<bool>],
        budget: &Budget,
        peer_only: bool,
    ) -> Option<Segment> {
        let sources = self.adjacency[to]
            .iter()
            .map(|&(id, _)| Source::Peer(id))
            .chain((!peer_only).then_some(Source::S3));
        let mut best: Option<((usize, usize, usize, usize), Segment)> = None;
        for source in sources {
            let Some(mib) = self.possible(slot, source, to, chunk, owners, budget) else {
                continue;
            };
            if mib == 0 {
                continue;
            }
            let id = match source {
                Source::Peer(peer) => peer,
                Source::S3 => self.network.nodes.len(),
            };
            let is_s3 = usize::from(source == Source::S3);
            let key = match policy {
                Policy::S3Only => {
                    if source != Source::S3 {
                        continue;
                    }
                    (0, 0, 0, 0)
                }
                Policy::PeerFirst => (is_s3, id, 0, 0),
                Policy::ForecastDirect | Policy::AdaptiveRelay => (
                    self.predicted_finish(slot, source, to, chunk, deadline),
                    is_s3,
                    usize::MAX - mib,
                    id,
                ),
            };
            let seg = Segment { slot, from: source, to, chunk, mib };
            if best.as_ref().is_none_or(|(old, _)| key < *old) {
                best = Some((key, seg));
            }
        }
        best.map(|(_, seg)| seg)
    }

    fn relays_for(&self, receiver: usize, chunk: usize, slot: usize, deadline: usize) -> Vec<usize> {
        self.adjacency[receiver]
            .iter()
            .map(|&(peer, _)| peer)
            .filter(|&peer| {
                peer != receiver
                    && self.received[peer][chunk] < self.network.sizes_mib[chunk]
                    && self.network.nodes[peer].prefetch_opt_in
                    && self.network.nodes[peer].share_enabled
                    && (slot + 1..=deadline).any(|later| {
                        self.network.nodes[peer].online[later]
                            && self.network.nodes[receiver].online[later]
                    })
            })
            .collect()
    }
}

/// A fault-tolerant, bounded greedy scheduler that inspects the actual network
/// only at the current slot, and replans once per slot. The future is read
/// solely from the forecast in Network.nodes[*].online.
///
/// It treats already-received verified MiB as persistent across interruptions.
/// This is a simplified independent-MiB-verification model, not a transport
/// implementation, Bao proof verifier, or grant.
pub fn simulate(
    network: &Network,
    actual: &[Vec<bool>],
    policy: Policy,
) -> Result<Plan, String> {
    let mut state = Working::new(network, actual)?;
    let mut requests = network.requests.clone();
    requests.sort_by_key(|r| (r.last_slot, r.receiver, r.chunk));
    for slot in 0..network.steps {
        let owners = state.full_snapshot();
        let mut budget = Budget::new(network);
        // Requested parts have priority over speculative relay prefetch.
        for req in &requests {
            if slot > req.last_slot {
                continue;
            }
            if let Some(segment) = state.select(
                policy,
                slot,
                req.receiver,
                req.chunk,
                req.last_slot,
                &owners,
                &budget,
                false,
            ) {
                state.record(segment, &mut budget)?;
            }
        }
        // The only speculative operation allowed is a voluntary public-chunk
        // prefetch into a consenting intermediary that could later serve a
        // currently pending request. Never prefetch from S3 in this prototype.
        if policy == Policy::AdaptiveRelay {
            for req in &requests {
                if slot >= req.last_slot
                    || state.received[req.receiver][req.chunk] == network.sizes_mib[req.chunk]
                {
                    continue;
                }
                for relay in state.relays_for(req.receiver, req.chunk, slot, req.last_slot) {
                    if let Some(segment) = state.select(
                        policy,
                        slot,
                        relay,
                        req.chunk,
                        req.last_slot,
                        &owners,
                        &budget,
                        true,
                    ) {
                        state.record(segment, &mut budget)?;
                    }
                }
            }
        }
    }
    let result = Plan { metrics: state.summary(), segments: state.segments };
    result.verify(network, actual)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::{Link, Node, Request};

    fn interrupted_peer() -> (Network, Vec<Vec<bool>>) {
        let network = Network {
            steps: 5,
            sizes_mib: vec![5],
            s3_rate_mib_per_slot: 1,
            s3_parallel: 1,
            nodes: vec![
                Node {
                    online: vec![true; 5],
                    cached: vec![true],
                    upload_parallel: 1,
                    download_parallel: 1,
                    share_enabled: true,
                    prefetch_opt_in: false,
                },
                Node {
                    online: vec![true; 5],
                    cached: vec![false],
                    upload_parallel: 1,
                    download_parallel: 1,
                    share_enabled: false,
                    prefetch_opt_in: false,
                },
            ],
            links: vec![Link { a: 0, b: 1, rate_mib_per_slot: 2 }],
            requests: vec![Request { receiver: 1, chunk: 0, last_slot: 4 }],
        };
        let mut actual = network.nodes.iter().map(|n| n.online.clone()).collect::<Vec<_>>();
        actual[0][2] = false;
        actual[0][3] = false;
        actual[0][4] = false;
        (network, actual)
    }

    #[test]
    fn source_switch_resumes_only_missing_verified_segments() {
        let (network, actual) = interrupted_peer();
        let result = simulate(&network, &actual, Policy::PeerFirst).unwrap();
        assert_eq!(result.metrics.completed, 1);
        assert_eq!(result.metrics.p2p_mib, 4);
        assert_eq!(result.metrics.s3_mib, 1);
        assert_eq!(result.metrics.source_switches, 1);
        result.verify(&network, &actual).unwrap();
    }

    #[test]
    fn no_same_slot_relay_of_incomplete_or_newly_complete_chunk() {
        let mut network = interrupted_peer().0;
        network.steps = 2;
        for n in &mut network.nodes { n.online.truncate(2); }
        network.nodes.push(Node {
            online: vec![true; 2],
            cached: vec![false],
            upload_parallel: 1,
            download_parallel: 1,
            share_enabled: false,
            prefetch_opt_in: false,
        });
        network.links.push(Link { a: 1, b: 2, rate_mib_per_slot: 5 });
        network.nodes[1].prefetch_opt_in = true;
        network.requests = vec![Request { receiver: 2, chunk: 0, last_slot: 1 }];
        network.s3_parallel = 0;
        let actual = network.nodes.iter().map(|n| n.online.clone()).collect::<Vec<_>>();
        let result = simulate(&network, &actual, Policy::AdaptiveRelay).unwrap();
        assert_eq!(result.metrics.completed, 0);
    }

    #[test]
    fn no_prefetch_without_optin() {
        let mut network = interrupted_peer().0;
        network.nodes.push(Node {
            online: vec![true; 5],
            cached: vec![false],
            upload_parallel: 1,
            download_parallel: 1,
            share_enabled: true,
            prefetch_opt_in: false,
        });
        network.links.push(Link { a: 0, b: 2, rate_mib_per_slot: 5 });
        network.links.push(Link { a: 2, b: 1, rate_mib_per_slot: 5 });
        let actual = network.nodes.iter().map(|n| n.online.clone()).collect::<Vec<_>>();
        let result = simulate(&network, &actual, Policy::AdaptiveRelay).unwrap();
        assert!(result.segments.iter().all(|tr| tr.to != 2));
    }

    #[test]
    fn tampered_mib_and_outage_fail_replay() {
        let (network, actual) = interrupted_peer();
        let mut result = simulate(&network, &actual, Policy::PeerFirst).unwrap();
        result.segments[0].mib += 1;
        assert!(result.verify(&network, &actual).is_err());
        let mut result = simulate(&network, &actual, Policy::PeerFirst).unwrap();
        result.segments[0].slot = 2;
        assert!(result.verify(&network, &actual).is_err());
    }

    #[test]
    fn actual_trace_must_have_exact_dimensions() {
        let (network, mut actual) = interrupted_peer();
        actual[1].pop();
        assert!(simulate(&network, &actual, Policy::S3Only).is_err());
    }

    #[test]
    fn no_delivery_after_deadline() {
        let (mut network, actual) = interrupted_peer();
        network.requests[0].last_slot = 0;
        let result = simulate(&network, &actual, Policy::PeerFirst).unwrap();
        assert_eq!(result.metrics.completed, 0);
    }

    #[test]
    fn simulation_is_deterministic_for_same_observation() {
        let (network, actual) = interrupted_peer();
        let a = simulate(&network, &actual, Policy::AdaptiveRelay).unwrap();
        let b = simulate(&network, &actual, Policy::AdaptiveRelay).unwrap();
        assert_eq!(a.segments, b.segments);
        assert_eq!(a.metrics, b.metrics);
    }
}
