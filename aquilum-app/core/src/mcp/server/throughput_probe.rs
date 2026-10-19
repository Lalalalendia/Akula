//! Opt-in MCP read-only synthetic throughput diagnostic.
//! Run: cargo test -p aquilum-core mcp::server::tests::throughput_probe -- --ignored --nocapture
//! This is NOT an agent workload benchmark or authorization to parallelize writes.
use super::*;
use std::sync::{Arc, Barrier};
use std::time::Instant;

#[test]
#[ignore = "opt-in timing diagnostic, not a CI performance assertion"]
fn baseline_serialized_tool_latency_at_1_4_8_16_clients() {
    for clients in [1usize, 4, 8, 16] {
        const REQUESTS_PER_CLIENT: usize = 4;
        const SYNTHETIC_TOOL_MILLIS: u64 = 12;
        let tool = slow_tool(Duration::from_millis(SYNTHETIC_TOOL_MILLIS));
        let (server, url) = start(tool);
        let gate = Arc::new(Barrier::new(clients + 1));
        let threads = (0..clients).map(|_| {
            let url = url.clone();
            let gate = Arc::clone(&gate);
            thread::spawn(move || {
                gate.wait();
                let mut durations = Vec::with_capacity(REQUESTS_PER_CLIENT);
                for _ in 0..REQUESTS_PER_CLIENT {
                    let started = Instant::now();
                    let response = ureq::post(&url)
                        .set("Authorization", &bearer())
                        .send_string(&tool_call("synthetic-read"));
                    assert!(response.is_ok_and(|r| r.status() == 200));
                    durations.push(started.elapsed().as_micros());
                }
                durations
            })
        }).collect::<Vec<_>>();
        let total_start = Instant::now();
        gate.wait();
        let mut samples = threads.into_iter()
            .flat_map(|t| t.join().unwrap()).collect::<Vec<_>>();
        let elapsed = total_start.elapsed();
        samples.sort_unstable();
        let requests = samples.len();
        let p50 = samples[(requests - 1) / 2];
        let p95 = samples[((requests * 95).div_ceil(100) - 1).min(requests - 1)];
        let rate = requests as f64 / elapsed.as_secs_f64();
        eprintln!(
            "MCP_SYNTHETIC_SERIAL_BASELINE clients={clients} requests={requests}              tool_delay_ms={SYNTHETIC_TOOL_MILLIS} elapsed_ms={:.1}              req_per_sec={rate:.1} p50_us={p50} p95_us={p95}",
            elapsed.as_secs_f64() * 1000.0
        );
        server.stop();
    }
}
