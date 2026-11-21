//! Deterministic stress study for the separate variable-duration model.
//! The online trace is synthetic; no P2P or S3 operations occur.
use std::collections::HashSet;
use std::time::Instant;

use akula_p2p_temporal_scheduler_lab::duration::{
    evaluate_observed, schedule, Link, Network, Node, Request, Strategy,
};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }

    fn chance(&mut self, numerator: usize, denominator: usize) -> bool {
        self.pick(denominator) < numerator
    }
}

fn instance(rng: &mut Rng) -> Network {
    let n = 5 + rng.pick(8);
    let chunks = 1 + rng.pick(4);
    let steps = 12 + rng.pick(13);
    let sizes_mib = (0..chunks).map(|_| 1 + rng.pick(12)).collect();
    let mut nodes = Vec::new();
    for id in 0..n {
        let phase = rng.pick(8);
        let period = 4 + rng.pick(7);
        let duty = 2 + rng.pick(period - 1);
        let online = (0..steps)
            .map(|t| (id == 0 || (t + phase) % period < duty) && rng.chance(15, 16))
            .collect();
        let cached = (0..chunks)
            .map(|chunk| id == (chunk % n) || rng.chance(1, 8))
            .collect();
        nodes.push(Node {
            online,
            cached,
            upload_parallel: 1 + rng.pick(2),
            download_parallel: 1 + rng.pick(2),
            share_enabled: rng.chance(3, 4),
            prefetch_opt_in: rng.chance(1, 2),
        });
    }

    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for a in 0..n {
        let b = (a + 1) % n;
        let pair = (a.min(b), a.max(b));
        if seen.insert(pair) {
            links.push(Link {
                a, b,
                rate_mib_per_slot: [1, 2, 4, 8][rng.pick(4)],
            });
        }
    }
    for a in 0..n {
        for b in a + 2..n {
            let pair = (a.min(b), a.max(b));
            if seen.insert(pair) && rng.chance(1, 3) {
                links.push(Link {
                    a, b,
                    rate_mib_per_slot: [1, 2, 4, 8][rng.pick(4)],
                });
            }
        }
    }
    let mut requests = Vec::new();
    for receiver in 0..n {
        for chunk in 0..chunks {
            if rng.chance(2, 3) {
                requests.push(Request {
                    receiver,
                    chunk,
                    last_slot: 5 + rng.pick(steps - 5),
                });
            }
        }
    }
    if requests.is_empty() {
        requests.push(Request {
            receiver: n - 1,
            chunk: 0,
            last_slot: steps - 1,
        });
    }
    Network {
        nodes,
        links,
        sizes_mib,
        steps,
        s3_rate_mib_per_slot: 2 + rng.pick(5),
        s3_parallel: 1 + rng.pick(3),
        requests,
    }
}

/// Same realized trace for all strategies on a given case and outage level.
fn observed(network: &Network, rng: &mut Rng, outage_percent: usize) -> Vec<Vec<bool>> {
    network.nodes.iter().map(|node| {
        node.online.iter().map(|&available| {
            available && !rng.chance(outage_percent, 100)
        }).collect()
    }).collect()
}

#[derive(Default)]
struct Totals {
    completed: usize,
    demands: usize,
    s3_mib: usize,
    peer_mib: usize,
    observed_10_completed: usize,
    observed_10_s3_mib: usize,
    observed_10_failed: usize,
    observed_25_completed: usize,
    observed_25_failed: usize,
    runtimes_us: Vec<u128>,
}

impl Totals {
    fn output(&mut self, name: &str) {
        self.runtimes_us.sort_unstable();
        let p95 = self.runtimes_us[self.runtimes_us.len() * 95 / 100];
        println!(
            "{name},{},{},{},{},{},{},{},{},{},{},{}",
            self.completed,
            self.demands,
            self.s3_mib,
            self.peer_mib,
            self.observed_10_completed,
            self.observed_10_s3_mib,
            self.observed_10_failed,
            self.observed_25_completed,
            self.observed_25_failed,
            self.runtimes_us.len(),
            p95
        );
    }
}

fn main() -> Result<(), String> {
    const CASES: usize = 120;
    let modes = [
        ("s3_only", Strategy::S3Only),
        ("greedy_peer_first", Strategy::GreedyOnDemand),
        ("forecast_direct", Strategy::WindowAwareDirect),
        ("temporal_multihop", Strategy::TemporalRoute),
    ];
    let mut reports = [
        Totals::default(),
        Totals::default(),
        Totals::default(),
        Totals::default(),
    ];
    let mut rnd = Rng(0xa31f_6825_2026_b0aa);
    for _case_id in 0..CASES {
        let network = instance(&mut rnd);
        network.validate()?;
        let actual10 = observed(&network, &mut rnd, 10);
        let actual25 = observed(&network, &mut rnd, 25);
        for ((_, mode), report) in modes.iter().zip(reports.iter_mut()) {
            let before = Instant::now();
            let plan = schedule(&network, *mode)?;
            let elapsed = before.elapsed().as_micros();
            plan.verify(&network)?;
            let out10 = evaluate_observed(&network, &plan, &actual10)?;
            let out25 = evaluate_observed(&network, &plan, &actual25)?;
            report.completed += plan.metrics.completed;
            report.demands += plan.metrics.total;
            report.s3_mib += plan.metrics.s3_mib;
            report.peer_mib += plan.metrics.peer_mib;
            report.observed_10_completed += out10.completed;
            report.observed_10_s3_mib += out10.attempted_s3_mib;
            report.observed_10_failed += out10.failed_offline + out10.failed_dependency;
            report.observed_25_completed += out25.completed;
            report.observed_25_failed += out25.failed_offline + out25.failed_dependency;
            report.runtimes_us.push(elapsed);
        }
    }

    println!("DURATION_SIM_CASES={CASES}");
    println!("DURATION_SIM_SEED=a31f68252026b0aa");
    println!("SYNTHETIC_ONLY variable-size MiB chunks, integer link rates, no retries");
    println!("forecast_10/25 are observed offline drop rates with NO replanning");
    println!("strategy,planned_completed,demands,planned_s3_mib,planned_peer_mib,observed10_completed,observed10_s3_attempted_mib,observed10_failed,observed25_completed,observed25_failed,cases,planner_p95_us");
    for ((name, _), report) in modes.iter().zip(reports.iter_mut()) {
        report.output(name);
    }
    Ok(())
}
