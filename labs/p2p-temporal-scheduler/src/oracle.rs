//! Exact small-instance upper bound for the offline one-chunk scheduling lab.
//! Exponential search with (slot, verified-cache bitmask) memoization.
//! This is an oracle for tiny synthetic tests, not a production scheduler.

use std::collections::HashMap;

use crate::Scenario;

/// Lexicographic objective: maximize unique demands completed before deadlines,
/// then minimize S3 transfers among equally successful schedules.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Optimum {
    pub completed: usize,
    pub s3_chunks: usize,
}

impl Optimum {
    fn better_than(self, other: Self) -> bool {
        self.completed > other.completed
            || (self.completed == other.completed && self.s3_chunks < other.s3_chunks)
    }
}

struct Oracle<'a> {
    s: &'a Scenario,
    adjacency: Vec<Vec<usize>>,
    deadlines: Vec<Option<usize>>,
    cache: HashMap<(usize, u32), Optimum>,
}

impl Oracle<'_> {
    fn solve(&mut self, slot: usize, mask: u32) -> Optimum {
        if slot == self.s.horizon {
            return Optimum::default();
        }
        if let Some(&score) = self.cache.get(&(slot, mask)) {
            return score;
        }

        // Enumerate all compatible transfers that start in this slot. Senders
        // are determined by the old mask: same-slot forwarding is forbidden.
        let mut used_uploads = vec![0usize; self.s.peers.len()];
        let mut best = Optimum {
            completed: 0,
            s3_chunks: usize::MAX,
        };
        self.enumerate(
            slot,
            mask,
            0,
            mask,
            0,
            &mut used_uploads,
            &mut best,
        );
        self.cache.insert((slot, mask), best);
        best
    }

    #[allow(clippy::too_many_arguments)]
    fn enumerate(
        &mut self,
        slot: usize,
        old_mask: u32,
        target: usize,
        new_mask: u32,
        s3_used: usize,
        used_uploads: &mut [usize],
        best: &mut Optimum,
    ) {
        if target == self.s.peers.len() {
            let future = self.solve(slot + 1, new_mask);
            let mut reward = 0;
            for (peer, deadline) in self.deadlines.iter().enumerate() {
                if let Some(last) = deadline {
                    let bit = 1u32 << peer;
                    if slot <= *last && old_mask & bit == 0 && new_mask & bit != 0 {
                        reward += 1;
                    }
                }
            }
            let candidate = Optimum {
                completed: reward + future.completed,
                s3_chunks: s3_used + future.s3_chunks,
            };
            if candidate.better_than(*best) {
                *best = candidate;
            }
            return;
        }

        // Not delivering to this peer is always an option.
        self.enumerate(
            slot,
            old_mask,
            target + 1,
            new_mask,
            s3_used,
            used_uploads,
            best,
        );

        let bit = 1u32 << target;
        let receiver = &self.s.peers[target];
        if old_mask & bit != 0
            || !receiver.online[slot]
            || receiver.download_slots == 0
        {
            return;
        }

        if s3_used < self.s.s3_upload_slots {
            self.enumerate(
                slot,
                old_mask,
                target + 1,
                new_mask | bit,
                s3_used + 1,
                used_uploads,
                best,
            );
        }

        for sender in 0..self.s.peers.len() {
            let peer = &self.s.peers[sender];
            if old_mask & (1u32 << sender) == 0
                || !peer.online[slot]
                || !peer.share_enabled
                || used_uploads[sender] >= peer.upload_slots
                || !self.adjacency[sender].contains(&target)
            {
                continue;
            }
            used_uploads[sender] += 1;
            self.enumerate(
                slot,
                old_mask,
                target + 1,
                new_mask | bit,
                s3_used,
                used_uploads,
                best,
            );
            used_uploads[sender] -= 1;
        }
    }
}

/// Exhaustive reference optimum for at most five peers, six slots and one
/// equal-size chunk. These limits are intentional: complexity is exponential.
pub fn exact_one_chunk(s: &Scenario) -> Result<Optimum, String> {
    s.validate()?;
    if s.chunks != 1 || s.peers.len() > 5 || s.horizon > 6 {
        return Err("exact oracle supports one chunk, <=5 peers, <=6 slots".into());
    }
    let initial_mask = s
        .peers
        .iter()
        .enumerate()
        .fold(0u32, |mask, (index, peer)| {
            mask | if peer.cached[0] { 1u32 << index } else { 0 }
        });

    let mut deadlines = vec![None; s.peers.len()];
    for demand in &s.demands {
        deadlines[demand.receiver] = Some(demand.deadline);
    }
    let initially_completed = s
        .demands
        .iter()
        .filter(|d| s.peers[d.receiver].cached[0])
        .count();

    let mut oracle = Oracle {
        s,
        adjacency: s.adjacency(),
        deadlines,
        cache: HashMap::new(),
    };
    let mut optimum = oracle.solve(0, initial_mask);
    optimum.completed += initially_completed;
    Ok(optimum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bridge_demo, schedule, Peer, Strategy};

    #[test]
    fn bridge_fixture_has_two_completed_without_s3() {
        assert_eq!(
            exact_one_chunk(&bridge_demo()).unwrap(),
            Optimum {
                completed: 2,
                s3_chunks: 0
            }
        );
    }

    #[test]
    fn oracle_rejects_oversized_instances() {
        let mut s = bridge_demo();
        s.horizon = 7;
        for p in &mut s.peers {
            p.online.resize(7, false);
        }
        assert!(exact_one_chunk(&s).is_err());
    }

    #[test]
    fn initially_cached_demands_are_free() {
        let mut s = bridge_demo();
        s.demands[0].receiver = 0;
        s.demands[1].receiver = 1;
        assert_eq!(exact_one_chunk(&s).unwrap().completed, 2);
    }

    // Fixed seed: this should remain stable and not rely on a network or
    // any platform-specific random source.
    #[test]
    fn eighty_tiny_cases_never_beat_the_exact_oracle() {
        let mut seed = 0x2468_ace0_1357_bdf1_u64;
        fn rnd(seed: &mut u64, modulo: usize) -> usize {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed as usize) % modulo
        }

        for case_id in 0..80 {
            let n = 2 + rnd(&mut seed, 3);
            let h = 2 + rnd(&mut seed, 4);
            let mut peers = Vec::new();
            for id in 0..n {
                let cached = id == 0 || rnd(&mut seed, 5) == 0;
                let online = (0..h).map(|_| rnd(&mut seed, 4) != 0).collect();
                peers.push(Peer {
                    online,
                    cached: vec![cached],
                    upload_slots: rnd(&mut seed, 3),
                    download_slots: 1,
                    share_enabled: rnd(&mut seed, 4) != 0,
                });
            }
            let mut links = Vec::new();
            for a in 0..n {
                for b in a + 1..n {
                    if rnd(&mut seed, 3) != 0 {
                        links.push((a, b));
                    }
                }
            }
            let mut demands = Vec::new();
            for receiver in 0..n {
                if rnd(&mut seed, 3) != 0 {
                    demands.push(crate::Demand {
                        receiver,
                        chunk: 0,
                        deadline: rnd(&mut seed, h),
                    });
                }
            }
            let s = Scenario {
                peers,
                links,
                chunks: 1,
                horizon: h,
                s3_upload_slots: rnd(&mut seed, 3),
                demands,
            };
            let optimum = exact_one_chunk(&s).unwrap();
            for strategy in [
                Strategy::S3Only,
                Strategy::GreedyOnDemand,
                Strategy::TemporalRoute,
            ] {
                let outcome = schedule(&s, strategy).unwrap();
                assert!(
                    outcome.completed <= optimum.completed,
                    "case {case_id}: {strategy:?} completed {} > oracle {}",
                    outcome.completed,
                    optimum.completed
                );
                if outcome.completed == optimum.completed {
                    assert!(
                        outcome.s3_chunks >= optimum.s3_chunks,
                        "case {case_id}: {strategy:?} used fewer S3 chunks than the oracle"
                    );
                }
            }
        }
    }
}
