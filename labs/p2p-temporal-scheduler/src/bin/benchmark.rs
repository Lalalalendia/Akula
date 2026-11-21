//! Reproducible synthetic A/B benchmark (not a production network test).
use std::time::Instant;

use akula_p2p_temporal_scheduler_lab::{
    oracle::exact_one_chunk, schedule, Demand, Peer, Scenario, Strategy,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick(&mut self, exclusive: usize) -> usize {
        (self.next() as usize) % exclusive
    }

    fn chance(&mut self, numerator: usize, denominator: usize) -> bool {
        self.pick(denominator) < numerator
    }
}

fn make_scenario(rng: &mut Rng, id: usize) -> Scenario {
    let n = 4 + rng.pick(9);
    let chunks = 1 + rng.pick(3);
    let horizon = 5 + rng.pick(10);
    let peers = (0..n)
        .map(|peer_id| Peer {
            online: (0..horizon).map(|_| rng.chance(2 + (id % 2), 4)).collect(),
            cached: (0..chunks)
                .map(|chunk| (peer_id == chunk % n) || rng.chance(1, 8))
                .collect(),
            upload_slots: 1 + rng.pick(3),
            download_slots: 1 + rng.pick(2),
            share_enabled: rng.chance(4, 5),
        })
        .collect();

    let mut links = Vec::new();
    for a in 0..n {
        for b in a + 1..n {
            if rng.chance(1 + (id % 3), 4) {
                links.push((a, b));
            }
        }
    }
    let mut demands = Vec::new();
    for receiver in 0..n {
        for chunk in 0..chunks {
            if rng.chance(2, 3) {
                demands.push(Demand {
                    receiver,
                    chunk,
                    deadline: rng.pick(horizon),
                });
            }
        }
    }
    if demands.is_empty() {
        demands.push(Demand {
            receiver: n - 1,
            chunk: 0,
            deadline: horizon - 1,
        });
    }
    Scenario {
        peers,
        links,
        chunks,
        horizon,
        s3_upload_slots: 1 + rng.pick(3),
        demands,
    }
}

#[derive(Default)]
struct Summary {
    demands: usize,
    completed: usize,
    s3_chunks: usize,
    peer_chunks: usize,
    runtime_us: Vec<u128>,
}

impl Summary {
    fn record(&mut self, result: &akula_p2p_temporal_scheduler_lab::Outcome, micros: u128) {
        self.demands += result.total;
        self.completed += result.completed;
        self.s3_chunks += result.s3_chunks;
        self.peer_chunks += result.peer_chunks;
        self.runtime_us.push(micros);
    }

    fn report(&mut self, name: &str) {
        self.runtime_us.sort_unstable();
        let count = self.runtime_us.len();
        let p50 = self.runtime_us[count / 2];
        let p95 = self.runtime_us[(count * 95 / 100).min(count - 1)];
        let completion_pct = 100.0 * self.completed as f64 / self.demands.max(1) as f64;
        println!(
            "{name},{},{},{},{:.2},{},{},{},{}",
            count,
            self.completed,
            self.demands,
            completion_pct,
            self.s3_chunks,
            self.peer_chunks,
            p50,
            p95
        );
    }
}

fn main() -> Result<(), String> {
    let strategies = [
        ("s3_only", Strategy::S3Only),
        ("greedy_on_demand", Strategy::GreedyOnDemand),
        ("temporal_route", Strategy::TemporalRoute),
    ];
    let mut stats = [Summary::default(), Summary::default(), Summary::default()];
    let mut rng = Rng(0x5a17_82e5_abcd_0123);
    const CASES: usize = 120;
    for id in 0..CASES {
        let s = make_scenario(&mut rng, id);
        s.validate()?;
        for ((_, strategy), summary) in strategies.iter().zip(stats.iter_mut()) {
            let started = Instant::now();
            let result = schedule(&s, *strategy)?;
            let elapsed = started.elapsed().as_micros();
            result.verify(&s)?;
            summary.record(&result, elapsed);
        }
    }

    println!("AK_P2P_TEMPORAL_BENCH_SEED=5a1782e5abcd0123");
    println!("AK_P2P_TEMPORAL_BENCH_CASES={CASES}");
    println!("strategy,cases,completed,demands,completion_pct,s3_chunks,p2p_chunks,p50_us,p95_us");
    for ((name, _), summary) in strategies.iter().zip(stats.iter_mut()) {
        summary.report(name);
    }

    // Exact small-instance reference. This reports gaps, never passes a
    // heuristic off as an upper bound or a proof of optimality.
    let mut tiny = Rng(0x7a6c_1135_00ff_ee99);
    let mut optimum_sum = 0usize;
    let mut temporal_sum = 0usize;
    let mut exact_ties = 0usize;
    const TINY_CASES: usize = 30;
    for id in 0..TINY_CASES {
        let mut s = make_scenario(&mut tiny, id);
        let n = 3 + tiny.pick(2);
        s.peers.truncate(n);
        s.links.retain(|&(a, b)| a < n && b < n);
        s.chunks = 1;
        s.horizon = 3 + tiny.pick(3);
        for peer in &mut s.peers {
            peer.online.truncate(s.horizon);
            peer.cached.truncate(1);
        }
        let mut demands = Vec::new();
        for receiver in 0..n {
            if tiny.chance(2, 3) {
                demands.push(Demand {
                    receiver,
                    chunk: 0,
                    deadline: tiny.pick(s.horizon),
                });
            }
        }
        s.demands = demands;
        let exact = exact_one_chunk(&s)?;
        let temporal = schedule(&s, Strategy::TemporalRoute)?;
        if temporal.completed > exact.completed {
            return Err(format!(
                "heuristic exceeded exact optimum in tiny case {id}"
            ));
        }
        optimum_sum += exact.completed;
        temporal_sum += temporal.completed;
        exact_ties += usize::from(
            temporal.completed == exact.completed && temporal.s3_chunks == exact.s3_chunks,
        );
    }
    println!(
        "AK_P2P_EXACT_ORACLE cases={TINY_CASES} optimal_completed={optimum_sum} temporal_completed={temporal_sum} exact_lexicographic_ties={exact_ties}"
    );
    println!("All results are synthetic, oracle-visibility and unit-slot assumptions apply.");
    Ok(())
}
