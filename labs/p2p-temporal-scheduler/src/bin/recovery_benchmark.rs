//! Reproducible, synthetic equal-trace comparison of recovery policies.
//! MiB are independent verified segments in a mock network, never real bytes.
use std::collections::HashSet;
use std::time::Instant;

use akula_p2p_temporal_scheduler_lab::duration::{
    evaluate_observed, schedule as fixed_schedule, Link, Network, Node, Request,
    Strategy as FixedStrategy,
};
use akula_p2p_temporal_scheduler_lab::recovery::{simulate, Policy};

struct Rng(u64);

impl Rng {
    fn pick(&mut self, upper: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 as usize) % upper
    }

    fn chance(&mut self, n: usize, d: usize) -> bool {
        self.pick(d) < n
    }
}

fn network(rng: &mut Rng) -> Network {
    let count = 5 + rng.pick(7);
    let chunks = 1 + rng.pick(3);
    let steps = 10 + rng.pick(12);
    let sizes_mib = (0..chunks).map(|_| 2 + rng.pick(9)).collect();
    let mut nodes = Vec::new();
    for id in 0..count {
        let period = 4 + rng.pick(7);
        let duty = 2 + rng.pick(period - 1);
        let phase = rng.pick(period);
        nodes.push(Node {
            online: (0..steps)
                .map(|t| (id == 0 || (t + phase) % period < duty) && rng.chance(15, 16))
                .collect(),
            cached: (0..chunks)
                .map(|chunk| id == chunk % count || rng.chance(1, 10))
                .collect(),
            upload_parallel: 1 + rng.pick(2),
            download_parallel: 1 + rng.pick(2),
            share_enabled: rng.chance(4, 5),
            prefetch_opt_in: rng.chance(1, 2),
        });
    }
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for a in 0..count {
        for b in a + 1..count {
            if b == a + 1 || (a == 0 && b == count - 1) || rng.chance(1, 3) {
                let pair = (a, b);
                if seen.insert(pair) {
                    links.push(Link {
                        a,
                        b,
                        rate_mib_per_slot: [1, 2, 4, 8][rng.pick(4)],
                    });
                }
            }
        }
    }
    let mut requests = Vec::new();
    for receiver in 0..count {
        for chunk in 0..chunks {
            if rng.chance(2, 3) {
                requests.push(Request {
                    receiver,
                    chunk,
                    last_slot: 4 + rng.pick(steps - 4),
                });
            }
        }
    }
    if requests.is_empty() {
        requests.push(Request {
            receiver: count - 1,
            chunk: 0,
            last_slot: steps - 1,
        });
    }
    Network {
        nodes,
        links,
        sizes_mib,
        steps,
        s3_rate_mib_per_slot: 1 + rng.pick(5),
        s3_parallel: 1 + rng.pick(2),
        requests,
    }
}

fn perturb(n: &Network, rng: &mut Rng, probability_percent: usize) -> Vec<Vec<bool>> {
    n.nodes
        .iter()
        .map(|peer| {
            peer.online
                .iter()
                .map(|&online| online && !rng.chance(probability_percent, 100))
                .collect()
        })
        .collect()
}

#[derive(Default)]
struct Totals {
    completed: usize,
    total: usize,
    s3_mib: usize,
    peer_mib: usize,
    changed_source: usize,
    prefetched_mib: usize,
    incomplete_mib: usize,
    timing: Vec<u128>,
}

fn main() -> Result<(), String> {
    let policies = [
        ("s3_resume", Policy::S3Only),
        ("peer_first_resume", Policy::PeerFirst),
        ("forecast_direct_resume", Policy::ForecastDirect),
        ("adaptive_relay_resume", Policy::AdaptiveRelay),
    ];
    let levels = [0usize, 10, 25];
    let mut totals: Vec<Vec<Totals>> = (0..levels.len())
        .map(|_| (0..policies.len()).map(|_| Totals::default()).collect())
        .collect();
    let mut fixed_completed = [0usize; 3];
    let mut fixed_total = [0usize; 3];
    let mut rng = Rng(0x2026_a11c_0f77_5491);
    const CASES: usize = 120;
    for _case in 0..CASES {
        let n = network(&mut rng);
        n.validate()?;
        let fixed = fixed_schedule(&n, FixedStrategy::TemporalRoute)?;
        for (level_id, level) in levels.iter().enumerate() {
            let actual = perturb(&n, &mut rng, *level);
            let fixed_outcome = evaluate_observed(&n, &fixed, &actual)?;
            fixed_completed[level_id] += fixed_outcome.completed;
            fixed_total[level_id] += fixed_outcome.total;
            for (policy_id, (_, policy)) in policies.iter().enumerate() {
                let start = Instant::now();
                let plan = simulate(&n, &actual, *policy)?;
                let duration = start.elapsed().as_micros();
                plan.verify(&n, &actual)?;
                let group = &mut totals[level_id][policy_id];
                group.completed += plan.metrics.completed;
                group.total += plan.metrics.total;
                group.s3_mib += plan.metrics.s3_mib;
                group.peer_mib += plan.metrics.p2p_mib;
                group.changed_source += plan.metrics.source_switches;
                group.prefetched_mib += plan.metrics.prefetched_mib;
                group.incomplete_mib += plan.metrics.incomplete_mib;
                group.timing.push(duration);
            }
        }
    }
    println!("RECOVERY_SIM_CASES={CASES}");
    println!("RECOVERY_SIM_SEED=2026a11c0f775491");
    println!("synthetic per-MiB verified prefix model, no actual file/network/cryptography");
    println!("all policies see only current observed online status, future forecast is common");
    println!("fixed_temporal has no retries or resume and is provided for reference only");
    println!("unexpected_offline_pct,policy,completed,total,s3_verified_mib,p2p_verified_mib,source_switches,prefetch_mib,incomplete_mib,p95_calc_us");
    for (level_id, level) in levels.iter().enumerate() {
        println!(
            "{level},fixed_temporal_no_resume,{},{},,,,,,,",
            fixed_completed[level_id],
            fixed_total[level_id],
        );
        for (id, (label, _)) in policies.iter().enumerate() {
            let t = &mut totals[level_id][id];
            t.timing.sort_unstable();
            let p95 = t.timing[t.timing.len() * 95 / 100];
            println!(
                "{level},{label},{},{},{},{},{},{},{},{}",
                t.completed,
                t.total,
                t.s3_mib,
                t.peer_mib,
                t.changed_source,
                t.prefetched_mib,
                t.incomplete_mib,
                p95,
            );
        }
    }
    Ok(())
}
