//! Reproducible, synthetic equal-trace comparison of recovery policies.
//! MiB are independent verified segments in a mock network, never real bytes.
use std::collections::HashSet;
use std::time::Instant;

use akula_p2p_temporal_scheduler_lab::duration::{
    evaluate_observed, schedule as fixed_schedule, Link, Network, Node, Request,
    Strategy as FixedStrategy,
};
use akula_p2p_temporal_scheduler_lab::recovery::{
    simulate, simulate_with_mode, PartialMode, Policy,
};

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
    wasted_mib: usize,
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
    // Paired experiment: the same reactive policy and same observed trace.
    // The sole switched rule is whether independently verified partial MiB
    // persist across a pause or change of source.
    let compared_policies = [
        ("peer_first", Policy::PeerFirst),
        ("forecast_direct", Policy::ForecastDirect),
        ("adaptive_relay", Policy::AdaptiveRelay),
    ];
    let retention_modes = [
        ("keep_verified", PartialMode::KeepVerified),
        ("restart_interrupted", PartialMode::RestartInterrupted),
    ];
    let mut paired: Vec<Vec<Vec<Totals>>> = (0..levels.len())
        .map(|_| {
            (0..compared_policies.len())
                .map(|_| (0..retention_modes.len()).map(|_| Totals::default()).collect())
                .collect()
        })
        .collect();
    let mut paired_wins = vec![vec![[0usize; 3]; compared_policies.len()]; levels.len()];
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

            for (policy_id, (_, policy)) in compared_policies.iter().enumerate() {
                let mut pair_results = [0usize; 2];
                for (mode_id, (_, mode)) in retention_modes.iter().enumerate() {
                    let started = Instant::now();
                    let plan = simulate_with_mode(&n, &actual, *policy, *mode)?;
                    let elapsed = started.elapsed().as_micros();
                    plan.verify(&n, &actual)?;
                    pair_results[mode_id] = plan.metrics.completed;
                    let group = &mut paired[level_id][policy_id][mode_id];
                    group.completed += plan.metrics.completed;
                    group.total += plan.metrics.total;
                    group.s3_mib += plan.metrics.s3_mib;
                    group.peer_mib += plan.metrics.p2p_mib;
                    group.changed_source += plan.metrics.source_switches;
                    group.prefetched_mib += plan.metrics.prefetched_mib;
                    group.incomplete_mib += plan.metrics.incomplete_mib;
                    group.wasted_mib += plan.metrics.wasted_mib;
                    group.timing.push(elapsed);
                }
                let outcome = if pair_results[0] > pair_results[1] {
                    0
                } else if pair_results[0] == pair_results[1] {
                    1
                } else {
                    2
                };
                paired_wins[level_id][policy_id][outcome] += 1;
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
            fixed_completed[level_id], fixed_total[level_id],
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
    println!("PAIRED_RETENTION_AB_CASES={CASES}");
    println!("Only mode varies within a pair: retain verified partial MiB vs restart on interruption or source change");
    println!("offline_pct,policy,mode,completed,total,s3_mib,p2p_mib,wasted_mib,source_switches,p95_calc_us");
    for (level_id, level) in levels.iter().enumerate() {
        for (policy_id, (name, _)) in compared_policies.iter().enumerate() {
            for (mode_id, (mode_name, _)) in retention_modes.iter().enumerate() {
                let t = &mut paired[level_id][policy_id][mode_id];
                t.timing.sort_unstable();
                let p95 = t.timing[t.timing.len() * 95 / 100];
                println!(
                    "{level},{name},{mode_name},{},{},{},{},{},{},{}",
                    t.completed,
                    t.total,
                    t.s3_mib,
                    t.peer_mib,
                    t.wasted_mib,
                    t.changed_source,
                    p95,
                );
            }
            let w = paired_wins[level_id][policy_id];
            println!(
                "PAIRED_CASES offline_pct={level} policy={name} keep_better={} equal={} restart_better={}",
                w[0], w[1], w[2]
            );
        }
    }
    Ok(())
}
