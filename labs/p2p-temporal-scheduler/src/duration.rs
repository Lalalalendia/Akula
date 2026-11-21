//! Experimental duration-aware transfer planning for public, verified chunks.
//!
//! This is an offline synthetic model, NOT Akula transfer authorization.
//! Every transfer occupies a contiguous time interval (no resume/preemption).
//! Link rates and chunk sizes are integer MiB per time slot / MiB, respectively.
//! The predicted trace can be scored against a different observed online trace.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

const TIME_WEIGHT: u64 = 10_000;

#[derive(Clone, Debug)]
pub struct Node {
    pub online: Vec<bool>,
    pub cached: Vec<bool>,
    pub upload_parallel: usize,
    pub download_parallel: usize,
    pub share_enabled: bool,
    pub prefetch_opt_in: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    pub rate_mib_per_slot: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    pub receiver: usize,
    pub chunk: usize,
    /// Latest allowed transfer *occupancy* slot, inclusive.
    pub last_slot: usize,
}

#[derive(Clone, Debug)]
pub struct Network {
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub sizes_mib: Vec<usize>,
    pub steps: usize,
    pub s3_rate_mib_per_slot: usize,
    pub s3_parallel: usize,
    pub requests: Vec<Request>,
}

impl Network {
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() || self.steps == 0 || self.sizes_mib.is_empty()
            || self.s3_rate_mib_per_slot == 0
            || self.sizes_mib.contains(&0)
        {
            return Err("empty network, invalid sizes, or zero S3 rate".into());
        }
        let mut edges = HashSet::new();
        for link in &self.links {
            if link.a >= self.nodes.len()
                || link.b >= self.nodes.len()
                || link.a == link.b
                || link.rate_mib_per_slot == 0
                || !edges.insert((link.a.min(link.b), link.a.max(link.b)))
            {
                return Err("invalid or duplicate link".into());
            }
        }
        for node in &self.nodes {
            if node.online.len() != self.steps || node.cached.len() != self.sizes_mib.len() {
                return Err("invalid node online/cache dimensions".into());
            }
        }
        let mut unique = HashSet::new();
        for request in &self.requests {
            if request.receiver >= self.nodes.len()
                || request.chunk >= self.sizes_mib.len()
                || request.last_slot >= self.steps
                || !unique.insert((request.receiver, request.chunk))
            {
                return Err("invalid or duplicate request".into());
            }
        }
        Ok(())
    }

    fn neighbors(&self) -> Vec<Vec<(usize, usize)>> {
        let mut adj = vec![Vec::new(); self.nodes.len()];
        for (idx, link) in self.links.iter().enumerate() {
            adj[link.a].push((link.b, idx));
            adj[link.b].push((link.a, idx));
        }
        for v in &mut adj {
            v.sort_unstable();
        }
        adj
    }

    fn duration(&self, from: Source, to: usize, chunk: usize, adj: &[Vec<(usize, usize)>]) -> Option<usize> {
        let speed = match from {
            Source::S3 => self.s3_rate_mib_per_slot,
            Source::Peer(src) => adj.get(src)?.iter().find(|&&(peer, _)| peer == to)
                .map(|&(_, edge)| self.links[edge].rate_mib_per_slot)?,
        };
        Some(self.sizes_mib[chunk].div_ceil(speed))
    }

    fn link_id(&self, from: Source, to: usize, adj: &[Vec<(usize, usize)>]) -> Option<usize> {
        match from {
            Source::S3 => None,
            Source::Peer(src) => adj.get(src)?.iter().find(|&&(peer, _)| peer == to)
                .map(|&(_, edge)| edge),
        }
    }

    fn inbound_allowed(&self, to: usize, chunk: usize) -> bool {
        self.nodes[to].prefetch_opt_in
            || self.requests.iter().any(|r| r.receiver == to && r.chunk == chunk)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Source {
    S3,
    Peer(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transfer {
    pub start: usize,
    pub end: usize,
    pub from: Source,
    pub to: usize,
    pub chunk: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Strategy {
    S3Only,
    GreedyOnDemand,
    WindowAwareDirect,
    TemporalRoute,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Metrics {
    pub completed: usize,
    pub total: usize,
    pub s3_mib: usize,
    pub peer_mib: usize,
    pub last_delivery: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub transfers: Vec<Transfer>,
    pub metrics: Metrics,
}

impl Plan {
    /// Replay the entire schedule under the forecast to validate causality,
    /// capacity, complete-duration availability and source consent.
    pub fn verify(&self, network: &Network) -> Result<(), String> {
        network.validate()?;
        let adjacency = network.neighbors();
        let mut state = State::new(network);
        let mut transfers = self.transfers.clone();
        transfers.sort_by_key(|tr| (tr.start, tr.end, tr.to, tr.chunk));
        for transfer in transfers {
            if !state.source_ready(transfer.from, transfer.chunk, transfer.start) {
                return Err(format!("source did not possess chunk: {transfer:?}"));
            }
            if !state.can_fit(network, &adjacency, transfer) {
                return Err(format!("invalid interval/capacity/consent: {transfer:?}"));
            }
            state.reserve(network, &adjacency, transfer);
        }
        if state.metrics(network) != self.metrics {
            return Err("reported metrics differ from independent replay".into());
        }
        Ok(())
    }
}

struct State {
    ready: Vec<Vec<Option<usize>>>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
    edge_busy: Vec<Vec<usize>>,
    s3_busy: Vec<usize>,
    transfers: Vec<Transfer>,
}

impl State {
    fn new(network: &Network) -> Self {
        Self {
            ready: network.nodes.iter().map(|n| {
                n.cached.iter().map(|&present| present.then_some(0)).collect()
            }).collect(),
            outgoing: vec![vec![0; network.steps]; network.nodes.len()],
            incoming: vec![vec![0; network.steps]; network.nodes.len()],
            edge_busy: vec![vec![0; network.steps]; network.links.len()],
            s3_busy: vec![0; network.steps],
            transfers: Vec::new(),
        }
    }

    fn source_ready(&self, from: Source, chunk: usize, start: usize) -> bool {
        match from {
            Source::S3 => true,
            Source::Peer(src) => self.ready.get(src).and_then(|row| row.get(chunk))
                .and_then(|&ready| ready).is_some_and(|ready| ready <= start),
        }
    }

    /// Does not check source ownership: the time-expanded route finder may
    /// reach a node via a hypothetical earlier path. Reserve checks it later.
    fn can_fit(&self, network: &Network, adj: &[Vec<(usize, usize)>], tr: Transfer) -> bool {
        if tr.to >= network.nodes.len()
            || tr.chunk >= network.sizes_mib.len()
            || tr.start >= tr.end
            || tr.end > network.steps
            || self.ready[tr.to][tr.chunk].is_some()
            || !network.inbound_allowed(tr.to, tr.chunk)
        {
            return false;
        }
        let Some(duration) = network.duration(tr.from, tr.to, tr.chunk, adj) else {
            return false;
        };
        if duration != tr.end - tr.start {
            return false;
        }
        match tr.from {
            Source::S3 => {
                if network.s3_parallel == 0 {
                    return false;
                }
            }
            Source::Peer(src) => {
                if src == tr.to
                    || src >= network.nodes.len()
                    || !network.nodes[src].share_enabled
                    || network.nodes[src].upload_parallel == 0
                {
                    return false;
                }
            }
        }
        let link = network.link_id(tr.from, tr.to, adj);
        for t in tr.start..tr.end {
            if !network.nodes[tr.to].online[t]
                || self.incoming[tr.to][t] >= network.nodes[tr.to].download_parallel
            {
                return false;
            }
            match tr.from {
                Source::S3 => {
                    if self.s3_busy[t] >= network.s3_parallel {
                        return false;
                    }
                }
                Source::Peer(src) => {
                    if !network.nodes[src].online[t]
                        || self.outgoing[src][t] >= network.nodes[src].upload_parallel
                        || link.is_some_and(|edge| self.edge_busy[edge][t] != 0)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn reserve(&mut self, network: &Network, adj: &[Vec<(usize, usize)>], tr: Transfer) {
        debug_assert!(self.can_fit(network, adj, tr));
        debug_assert!(self.source_ready(tr.from, tr.chunk, tr.start));
        let link = network.link_id(tr.from, tr.to, adj);
        for t in tr.start..tr.end {
            self.incoming[tr.to][t] += 1;
            match tr.from {
                Source::S3 => self.s3_busy[t] += 1,
                Source::Peer(src) => {
                    self.outgoing[src][t] += 1;
                    if let Some(edge) = link {
                        self.edge_busy[edge][t] += 1;
                    }
                }
            }
        }
        self.ready[tr.to][tr.chunk] = Some(tr.end);
        self.transfers.push(tr);
    }

    fn metrics(&self, network: &Network) -> Metrics {
        let mut m = Metrics {
            total: network.requests.len(),
            ..Metrics::default()
        };
        for r in &network.requests {
            if let Some(ready) = self.ready[r.receiver][r.chunk] {
                if ready <= r.last_slot + 1 {
                    m.completed += 1;
                    m.last_delivery = Some(m.last_delivery.map_or(ready, |old: usize| old.max(ready)));
                }
            }
        }
        for tr in &self.transfers {
            match tr.from {
                Source::S3 => m.s3_mib += network.sizes_mib[tr.chunk],
                Source::Peer(_) => m.peer_mib += network.sizes_mib[tr.chunk],
            }
        }
        m
    }

    fn route(&self, network: &Network, adj: &[Vec<(usize, usize)>], req: Request) -> Option<Vec<Transfer>> {
        let peer_count = network.nodes.len();
        let count = peer_count + 1;
        let s3 = peer_count;
        let id = |t: usize, node: usize| t * count + node;
        let states = (network.steps + 1) * count;
        let mut dist = vec![u64::MAX; states];
        let mut prev: Vec<Option<(usize, Option<Transfer>)>> = vec![None; states];
        let mut heap = BinaryHeap::new();

        for node in 0..peer_count {
            if let Some(ready) = self.ready[node][req.chunk] {
                if ready <= req.last_slot + 1 {
                    let index = id(ready, node);
                    let cost = ready as u64 * TIME_WEIGHT;
                    if cost < dist[index] {
                        dist[index] = cost;
                        heap.push(Reverse((cost, ready, node)));
                    }
                }
            }
        }
        dist[id(0, s3)] = 0;
        heap.push(Reverse((0u64, 0usize, s3)));

        while let Some(Reverse((cost, t, node))) = heap.pop() {
            let current = id(t, node);
            if cost != dist[current] {
                continue;
            }
            if node == req.receiver && t <= req.last_slot + 1 {
                let mut path = Vec::new();
                let mut at = current;
                while let Some((parent, transfer)) = prev[at] {
                    if let Some(transfer) = transfer {
                        path.push(transfer);
                    }
                    at = parent;
                }
                path.reverse();
                return Some(path);
            }
            if t > req.last_slot {
                continue;
            }
            let wait = id(t + 1, node);
            if cost + TIME_WEIGHT < dist[wait] {
                dist[wait] = cost + TIME_WEIGHT;
                prev[wait] = Some((current, None));
                heap.push(Reverse((dist[wait], t + 1, node)));
            }

            let source = if node == s3 { Source::S3 } else { Source::Peer(node) };
            let mut visit = |target: usize| {
                let Some(duration) = network.duration(source, target, req.chunk, adj) else {
                    return;
                };
                let Some(end) = t.checked_add(duration) else {
                    return;
                };
                if end > req.last_slot + 1 {
                    return;
                }
                let transfer = Transfer { start: t, end, from: source, to: target, chunk: req.chunk };
                if !self.can_fit(network, adj, transfer) {
                    return;
                }
                let next = id(end, target);
                let penalty = if source == Source::S3 { 200 } else { 0 };
                let price = cost + duration as u64 * TIME_WEIGHT + penalty;
                if price < dist[next] {
                    dist[next] = price;
                    prev[next] = Some((current, Some(transfer)));
                    heap.push(Reverse((price, end, target)));
                }
            };
            if node == s3 {
                for target in 0..peer_count {
                    visit(target);
                }
            } else {
                for &(target, _) in &adj[node] {
                    visit(target);
                }
            }
        }
        None
    }
}

fn request_order(network: &Network) -> Vec<Request> {
    let mut requests = network.requests.clone();
    requests.sort_by_key(|req| (req.last_slot, req.receiver, req.chunk));
    requests
}

pub fn schedule(network: &Network, strategy: Strategy) -> Result<Plan, String> {
    network.validate()?;
    let adj = network.neighbors();
    let mut state = State::new(network);
    let requests = request_order(network);
    match strategy {
        Strategy::TemporalRoute => {
            for req in requests {
                if state.ready[req.receiver][req.chunk].is_some_and(|at| at <= req.last_slot + 1) {
                    continue;
                }
                if let Some(path) = state.route(network, &adj, req) {
                    for transfer in path {
                        if !state.source_ready(transfer.from, transfer.chunk, transfer.start)
                            || !state.can_fit(network, &adj, transfer)
                        {
                            return Err("non-committable speculative temporal route".into());
                        }
                        state.reserve(network, &adj, transfer);
                    }
                }
            }
        }
        Strategy::WindowAwareDirect => {
            for req in requests {
                if state.ready[req.receiver][req.chunk].is_some_and(|at| at <= req.last_slot + 1) {
                    continue;
                }
                let mut best: Option<(usize, bool, usize, usize, Transfer)> = None;
                for start in 0..=req.last_slot {
                    let sources = std::iter::once(Source::S3).chain(
                        adj[req.receiver].iter().map(|&(peer, _)| Source::Peer(peer))
                    );
                    for source in sources {
                        if !state.source_ready(source, req.chunk, start) {
                            continue;
                        }
                        let Some(duration) = network.duration(source, req.receiver, req.chunk, &adj) else {
                            continue;
                        };
                        let end = start + duration;
                        if end > req.last_slot + 1 {
                            continue;
                        }
                        let tr = Transfer { start, end, from: source, to: req.receiver, chunk: req.chunk };
                        if state.can_fit(network, &adj, tr) {
                            let key = (end, source == Source::S3, start, match source {
                                Source::Peer(peer) => peer,
                                Source::S3 => network.nodes.len(),
                            }, tr);
                            if best.as_ref().is_none_or(|b| key.0..=key.3 < b.0..=b.3) {
                                best = Some(key);
                            }
                        }
                    }
                }
                if let Some((_, _, _, _, tr)) = best {
                    state.reserve(network, &adj, tr);
                }
            }
        }
        Strategy::S3Only | Strategy::GreedyOnDemand => {
            for start in 0..network.steps {
                for &req in &requests {
                    if start > req.last_slot || state.ready[req.receiver][req.chunk].is_some() {
                        continue;
                    }
                    let mut chosen = None;
                    if strategy == Strategy::GreedyOnDemand {
                        for &(peer, _) in &adj[req.receiver] {
                            let source = Source::Peer(peer);
                            if !state.source_ready(source, req.chunk, start) {
                                continue;
                            }
                            let Some(duration) = network.duration(source, req.receiver, req.chunk, &adj) else {
                                continue;
                            };
                            let end = start + duration;
                            if end <= req.last_slot + 1 {
                                let tr = Transfer { start, end, from: source, to: req.receiver, chunk: req.chunk };
                                if state.can_fit(network, &adj, tr) {
                                    chosen = Some(tr);
                                    break;
                                }
                            }
                        }
                    }
                    if chosen.is_none() {
                        let end = start + network.sizes_mib[req.chunk].div_ceil(network.s3_rate_mib_per_slot);
                        if end <= req.last_slot + 1 {
                            let tr = Transfer { start, end, from: Source::S3, to: req.receiver, chunk: req.chunk };
                            if state.can_fit(network, &adj, tr) {
                                chosen = Some(tr);
                            }
                        }
                    }
                    if let Some(tr) = chosen {
                        state.reserve(network, &adj, tr);
                    }
                }
            }
        }
    }
    let plan = Plan { metrics: state.metrics(network), transfers: state.transfers };
    plan.verify(network)?;
    Ok(plan)
}

/// Realize a *fixed* planned schedule under unexpected offline slots. This
/// intentionally does NOT replan or retry after a failure; it measures forecast
/// brittleness, not a live end-to-end Akula implementation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Realized {
    pub completed: usize,
    pub total: usize,
    pub planned_transfers: usize,
    pub successful_transfers: usize,
    pub failed_offline: usize,
    pub failed_dependency: usize,
    pub attempted_s3_mib: usize,
    pub attempted_peer_mib: usize,
    pub delivered_s3_mib: usize,
    pub delivered_peer_mib: usize,
}

pub fn evaluate_observed(
    network: &Network,
    plan: &Plan,
    observed_online: &[Vec<bool>],
) -> Result<Realized, String> {
    plan.verify(network)?;
    if observed_online.len() != network.nodes.len()
        || observed_online.iter().any(|online| online.len() != network.steps)
    {
        return Err("invalid actual online trace".into());
    }
    let mut ready: Vec<Vec<Option<usize>>> = network.nodes.iter().map(|node| {
        node.cached.iter().map(|&cached| cached.then_some(0)).collect()
    }).collect();
    let mut transfers = plan.transfers.clone();
    transfers.sort_by_key(|tr| (tr.start, tr.end, tr.to, tr.chunk));
    let mut result = Realized { total: network.requests.len(), planned_transfers: transfers.len(), ..Realized::default() };
    for tr in transfers {
        let size = network.sizes_mib[tr.chunk];
        match tr.from {
            Source::S3 => result.attempted_s3_mib += size,
            Source::Peer(_) => result.attempted_peer_mib += size,
        }
        let owned = match tr.from {
            Source::S3 => true,
            Source::Peer(src) => ready[src][tr.chunk].is_some_and(|at| at <= tr.start),
        };
        if !owned {
            result.failed_dependency += 1;
            continue;
        }
        let online = (tr.start..tr.end).all(|t| {
            observed_online[tr.to][t]
                && match tr.from {
                    Source::S3 => true,
                    Source::Peer(src) => observed_online[src][t],
                }
        });
        if !online {
            result.failed_offline += 1;
            continue;
        }
        result.successful_transfers += 1;
        ready[tr.to][tr.chunk] = Some(tr.end);
        match tr.from {
            Source::S3 => result.delivered_s3_mib += size,
            Source::Peer(_) => result.delivered_peer_mib += size,
        }
    }
    for req in &network.requests {
        if ready[req.receiver][req.chunk].is_some_and(|at| at <= req.last_slot + 1) {
            result.completed += 1;
        }
    }
    Ok(result)
}

/// A 4-peer, 4 MiB chunk where two-slot transfers and a gap in bridge
/// availability require prepositioning into a consenting, caching node.
pub fn duration_bridge_demo() -> Network {
    Network {
        steps: 6,
        sizes_mib: vec![4],
        s3_parallel: 1,
        s3_rate_mib_per_slot: 2,
        nodes: vec![
            Node { online: vec![true, false, false, false, false, false], cached: vec![true], upload_parallel: 1, download_parallel: 1, share_enabled: true, prefetch_opt_in: false },
            Node { online: vec![true, false, false, true, true, false], cached: vec![false], upload_parallel: 2, download_parallel: 1, share_enabled: true, prefetch_opt_in: true },
            Node { online: vec![false, false, false, true, true, false], cached: vec![false], upload_parallel: 0, download_parallel: 1, share_enabled: false, prefetch_opt_in: false },
            Node { online: vec![false, false, false, true, true, false], cached: vec![false], upload_parallel: 0, download_parallel: 1, share_enabled: false, prefetch_opt_in: false },
        ],
        links: vec![
            Link { a: 0, b: 1, rate_mib_per_slot: 4 },
            Link { a: 1, b: 2, rate_mib_per_slot: 2 },
            Link { a: 1, b: 3, rate_mib_per_slot: 2 },
        ],
        requests: vec![
            Request { receiver: 2, chunk: 0, last_slot: 4 },
            Request { receiver: 3, chunk: 0, last_slot: 4 },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_routes_two_realistic_duration_transfers() {
        let network = duration_bridge_demo();
        let s3 = schedule(&network, Strategy::S3Only).unwrap();
        let greedy = schedule(&network, Strategy::GreedyOnDemand).unwrap();
        let direct = schedule(&network, Strategy::WindowAwareDirect).unwrap();
        let temporal = schedule(&network, Strategy::TemporalRoute).unwrap();
        assert_eq!(s3.metrics.completed, 1);
        assert_eq!(greedy.metrics.completed, 1);
        assert_eq!(direct.metrics.completed, 1);
        assert_eq!(temporal.metrics.completed, 2);
        assert_eq!(temporal.metrics.s3_mib, 0);
        assert_eq!(temporal.metrics.peer_mib, 12);
    }

    #[test]
    fn a_two_slot_transfer_requires_continuous_online_time() {
        let mut network = duration_bridge_demo();
        network.nodes[2].online[4] = false;
        network.s3_parallel = 0;
        let result = schedule(&network, Strategy::TemporalRoute).unwrap();
        assert_eq!(result.metrics.completed, 1);
    }

    #[test]
    fn an_unconsenting_bridge_must_not_receive_prefetch() {
        let mut network = duration_bridge_demo();
        network.nodes[1].prefetch_opt_in = false;
        network.s3_parallel = 0;
        let result = schedule(&network, Strategy::TemporalRoute).unwrap();
        assert_eq!(result.metrics.completed, 0);
    }

    #[test]
    fn a_peer_without_upload_consent_cannot_forward() {
        let mut network = duration_bridge_demo();
        network.nodes[1].share_enabled = false;
        network.s3_parallel = 0;
        let result = schedule(&network, Strategy::TemporalRoute).unwrap();
        assert_eq!(result.metrics.completed, 0);
    }

    #[test]
    fn lower_bandwidth_makes_deadline_unreachable() {
        let mut network = duration_bridge_demo();
        network.links[1].rate_mib_per_slot = 1;
        network.s3_parallel = 0;
        let result = schedule(&network, Strategy::TemporalRoute).unwrap();
        assert_eq!(result.metrics.completed, 1);
    }

    #[test]
    fn tampered_interval_is_rejected_by_replay() {
        let network = duration_bridge_demo();
        let mut plan = schedule(&network, Strategy::TemporalRoute).unwrap();
        plan.transfers[0].end += 1;
        assert!(plan.verify(&network).is_err());
    }

    #[test]
    fn offline_forecast_error_breaks_dependent_deliveries() {
        let network = duration_bridge_demo();
        let plan = schedule(&network, Strategy::TemporalRoute).unwrap();
        let mut actual: Vec<Vec<bool>> = network.nodes.iter().map(|n| n.online.clone()).collect();
        actual[1][0] = false;
        let observed = evaluate_observed(&network, &plan, &actual).unwrap();
        assert_eq!(observed.completed, 0);
        assert_eq!(observed.failed_offline, 1);
        assert_eq!(observed.failed_dependency, 2);
    }

    #[test]
    fn identical_traces_reproduce_planned_completion() {
        let network = duration_bridge_demo();
        let plan = schedule(&network, Strategy::TemporalRoute).unwrap();
        let actual: Vec<Vec<bool>> = network.nodes.iter().map(|n| n.online.clone()).collect();
        let observed = evaluate_observed(&network, &plan, &actual).unwrap();
        assert_eq!(observed.completed, plan.metrics.completed);
        assert_eq!(observed.delivered_peer_mib, plan.metrics.peer_mib);
    }

    #[test]
    fn invalid_rate_and_duplicate_link_are_rejected() {
        let mut network = duration_bridge_demo();
        network.links[0].rate_mib_per_slot = 0;
        assert!(network.validate().is_err());
        network.links[0].rate_mib_per_slot = 4;
        network.links.push(network.links[0]);
        assert!(network.validate().is_err());
    }
}
