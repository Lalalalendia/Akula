// Opt-in MCP synthetic throughput diagnostic.
// Run: cargo test -p aquilum-core mcp::server::tests::throughput_probe -- --ignored --nocapture
// NOT an agent workload benchmark or authorization to parallelize writes.
use super::*;
use std::sync::{Arc, Barrier};
use std::time::Instant;

#[test]
#[ignore = "opt-in timing diagnostic, not a CI performance assertion"]
fn baseline_serialized_tool_latency_at_1_4_8_16_clients() {
    const TOTAL_REQUESTS: usize = 128;
    const SYNTHETIC_TOOL_MILLIS: u64 = 12;

    for (route, tool_name) in [("exclusive", "synthetic-read"), ("read-only", "read_note")] {
        for clients in [1usize, 4, 8, 16] {
        assert_eq!(TOTAL_REQUESTS % clients, 0);
        let tool = slow_tool(Duration::from_millis(SYNTHETIC_TOOL_MILLIS));
        let (server, url) = start(tool);
        let gate = Arc::new(Barrier::new(clients + 1));
        let threads = (0..clients)
            .map(|_| {
                let url = url.clone();
                let gate = Arc::clone(&gate);
                thread::spawn(move || {
                    let agent = ureq::agent();
                    let authorization = bearer();
                    let request = tool_call(tool_name);
                    gate.wait();

                    let mut durations = Vec::with_capacity(TOTAL_REQUESTS / clients);
                    for _ in 0..TOTAL_REQUESTS / clients {
                        let started = Instant::now();
                        let response = agent
                            .post(&url)
                            .set("Authorization", &authorization)
                            .send_string(&request)
                            .expect("synthetic MCP request succeeded");
                        assert_eq!(response.status(), 200);
                        let body = response.into_string().expect("response body");
                        let json: Value = serde_json::from_str(&body).expect("MCP JSON response");
                        assert_eq!(json["id"], 2);
                        assert_eq!(json["result"]["isError"], false);
                        durations.push(started.elapsed().as_micros());
                    }
                    durations
                })
            })
            .collect::<Vec<_>>();

        gate.wait();
        let total_start = Instant::now();
        let mut samples = threads
            .into_iter()
            .flat_map(|worker| worker.join().expect("MCP worker"))
            .collect::<Vec<_>>();
        let elapsed = total_start.elapsed();
        samples.sort_unstable();
        assert_eq!(samples.len(), TOTAL_REQUESTS);

        let p50 = samples[(TOTAL_REQUESTS - 1) / 2];
        let p95 = samples[(TOTAL_REQUESTS * 95).div_ceil(100) - 1];
        let p99 = samples[(TOTAL_REQUESTS * 99).div_ceil(100) - 1];
        let rate = TOTAL_REQUESTS as f64 / elapsed.as_secs_f64();
        eprintln!(
            "MCP_SYNTHETIC_SERIAL_BASELINE route={route} clients={clients} requests={TOTAL_REQUESTS} \
             transport=keepalive tool_delay_ms={SYNTHETIC_TOOL_MILLIS} \
             elapsed_ms={:.1} req_per_sec={rate:.1} \
             p50_us={p50} p95_us={p95} p99_us={p99} errors=0",
            elapsed.as_secs_f64() * 1000.0
        );
        server.stop();
        }
    }
}
