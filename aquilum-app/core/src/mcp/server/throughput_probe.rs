// Opt-in MCP synthetic throughput diagnostic.
// Run: cargo test -p aquilum-core mcp::server::tests::throughput_probe -- --ignored --nocapture
// NOT an agent workload benchmark or authorization to parallelize writes.
use super::*;
use std::sync::{Arc, Barrier};
use std::time::Instant;

#[test]
#[ignore = "opt-in timing diagnostic, not a CI performance assertion"]
fn compare_exclusive_and_read_only_tool_latency_at_1_4_8_16_clients() {
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


#[test]
#[ignore = "opt-in real Markdown 10/32/100-client MCP workload, not a CI timing gate"]
fn burst_reads_against_real_temp_markdown_at_10_32_100_clients() {
    use crate::app_core::{Core, CoreEvent};
    use serde_json::json;
    use std::sync::atomic::Ordering;
    use std::fs;

    const NOTES: usize = 64;
    const READS_PER_CLIENT: usize = 6;
    const MAX_ATTEMPTS: usize = 75;

    let modes = if std::env::var("MCP_BURST_ATTEMPT").ok().as_deref() == Some("2") {
        [true, false]
    } else {
        [false, true]
    };
    for busy_grace in modes {
        for include_revision in [false, true] {
            for clients in [10usize, 32, 100] {
                // Exactly the real MCP read_note tool over disposable filesystem
                // documents, with no network access except loopback.
                let temp = tempfile::tempdir().expect("temp workspace");
                let appdata = temp.path().join("appdata");
                let root = temp.path().join("vault");
                fs::create_dir_all(&appdata).expect("appdata");
                fs::create_dir_all(&root).expect("vault");
                for note in 0..NOTES {
                    let path = root.join(format!("Note-{note:03}.md"));
                    let content = format!(
                        "# Note {note:03}\n\n{}",
                        "Local-only Markdown workload. No user data.\n".repeat(48)
                    );
                    fs::write(path, content).expect("write fixture note");
                }
                let root = fs::canonicalize(root).expect("canonical vault");
                let workspace = root.to_string_lossy().into_owned();
                let core = Core::open(&appdata, Arc::new(|_: CoreEvent| {}));
                core.ui_state
                    .resolve_workspace(&workspace, 1)
                    .expect("register fixture workspace");

                let tool_core = Arc::clone(&core);
                let (server, url) = start(Arc::new(move |name, arguments| {
                    crate::mcp::tools::call(&tool_core, name, arguments)
                }));
                server
                    .endpoint
                    .disable_busy_grace
                    .store(!busy_grace, Ordering::Relaxed);
                let gate = Arc::new(Barrier::new(clients + 1));
                let workers = (0..clients)
                    .map(|client_id| {
                        let url = url.clone();
                        let workspace = workspace.clone();
                        let gate = Arc::clone(&gate);
                        thread::spawn(move || {
                            let agent = ureq::agent();
                            let authorization = bearer();
                            let mut times = Vec::with_capacity(READS_PER_CLIENT);
                            let mut overload_503 = 0usize;
                            let mut transport_reset = 0usize;

                            gate.wait();
                            for read in 0..READS_PER_CLIENT {
                                let note = (client_id * 7 + read * 11) % NOTES;
                                let request = json!({
                                    "jsonrpc": "2.0",
                                    "id": 2,
                                    "method": "tools/call",
                                    "params": {
                                        "name": "read_note",
                                        "arguments": {
                                            "note": format!("Note-{note:03}.md"),
                                            "workspace": workspace,
                                            "includeRevision": include_revision,
                                        }
                                    },
                                }).to_string();
                                let started = Instant::now();
                                let mut done = false;

                                for attempt in 0..MAX_ATTEMPTS {
                                    match agent
                                        .post(&url)
                                        .set("Authorization", &authorization)
                                        .send_string(&request)
                                    {
                                        Ok(response) => {
                                            assert_eq!(response.status(), 200);
                                            let body = response.into_string().expect("response body");
                                            let result: Value = serde_json::from_str(&body)
                                                .expect("JSON-RPC body");
                                            assert_eq!(result["id"], 2);
                                            assert_eq!(result["result"]["isError"], false, "{result}");
                                            let value: Value = serde_json::from_str(
                                                result["result"]["content"][0]["text"]
                                                    .as_str()
                                                    .expect("read_note result text"),
                                            )
                                            .expect("read_note structured text");
                                            assert!(
                                                value["content"]
                                                    .as_str()
                                                    .expect("Markdown content")
                                                    .starts_with(&format!("# Note {note:03}\n")),
                                                "wrong Markdown file: {value}"
                                            );
                                            if include_revision {
                                                assert_eq!(
                                                    value["revision"].as_str().map(str::len),
                                                    Some(64)
                                                );
                                            } else {
                                                assert!(value.get("revision").is_none());
                                            }
                                            times.push(started.elapsed().as_micros());
                                            done = true;
                                            break;
                                        }
                                        Err(ureq::Error::Status(503, _)) => overload_503 += 1,
                                        Err(ureq::Error::Transport(_)) => transport_reset += 1,
                                        Err(error) => panic!("unexpected MCP read error: {error}"),
                                    }
                                    // Retrying a READ is safe. Deterministic per-agent
                                    // jitter avoids synchronized 100-client 503 storms.
                                    let backoff_ms = 5 + ((client_id * 13 + attempt * 7) % 19);
                                    thread::sleep(Duration::from_millis(backoff_ms as u64));
                                }
                                assert!(
                                    done,
                                    "client {client_id} failed to read Note-{note:03} after {MAX_ATTEMPTS} attempts"
                                );
                            }
                            (times, overload_503, transport_reset)
                        })
                    })
                    .collect::<Vec<_>>();

                gate.wait();
                let start = Instant::now();
                let mut samples = Vec::with_capacity(clients * READS_PER_CLIENT);
                let mut overload = 0usize;
                let mut resets = 0usize;
                for worker in workers {
                    let (durations, rejected, disconnected) =
                        worker.join().expect("agent worker completes");
                    samples.extend(durations);
                    overload += rejected;
                    resets += disconnected;
                }
                let elapsed = start.elapsed();
                samples.sort_unstable();
                assert_eq!(samples.len(), clients * READS_PER_CLIENT);

                let count = samples.len();
                let p50 = samples[(count - 1) / 2];
                let p95 = samples[(count * 95).div_ceil(100) - 1];
                let p99 = samples[(count * 99).div_ceil(100) - 1];
                let rate = count as f64 / elapsed.as_secs_f64();
                eprintln!(
                    "MCP_AGENT_BURST kind=real-markdown busy_grace={busy_grace} include_revision={include_revision} \
                     clients={clients} requests={count} successes={count} \
                     overload_503={overload} transport_resets={resets} exhausted=0 \
                     elapsed_ms={:.1} req_per_sec={rate:.1} \
                     p50_us={p50} p95_us={p95} p99_us={p99}",
                    elapsed.as_secs_f64() * 1000.0
                );
                server.stop();
                core.shutdown();
            }
        }
    }
}
