/// Isolated performance experiment. Never runs during normal unit tests.
/// Each process handles only ONE mode so peak RSS is interpretable.
/// The companion Python harness measures *this process* CPU/RSS, while
/// per-pair Instant timings exclude the Git history subprocesses.
#[test]
#[ignore = "Only run in dedicated DELTA-TEXT-04 Rust codec A/B workflow"]
fn real_git_markdown_pairs_mode_report() {
    use std::{env, path::Path, process::Command, time::Instant};

    let mode = env::var("AKULA_DELTA_RUST_BENCH_MODE")
        .expect("Choose full, splice, dictionary or selected mode");
    assert!(
        ["full", "splice", "dictionary", "selected"].contains(&mode.as_str()),
        "unknown codec benchmark mode"
    );
    let repetitions: usize = env::var("AKULA_DELTA_RUST_BENCH_REPS")
        .unwrap_or_else(|_| "7".to_owned())
        .parse()
        .expect("positive repetition count");
    assert!((1..=25).contains(&repetitions));

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap();
    let git = |args: &[&str]| -> Option<Vec<u8>> {
        let out = Command::new("git").current_dir(repo).args(args)
            .output().expect("Git is required for actual history");
        if out.status.success() { Some(out.stdout) } else { None }
    };
    // Same public Markdown paths and selection rule as DELTA-TEXT-02.
    let paths = [
        "docs/performance-roadmap.md",
        "README.ru.md",
        "docs/sync/adr-0006-cross-process-mutation-boundary.md",
        "docs/sync/adr-0005-inprocess-history-files-interlock.md",
        "aquilum-app/native/testdata/markdown-test.md",
    ];
    let mut total = 0_usize;
    let mut visited_paths = 0_usize;
    for path in paths {
        let history = git(&["log", "--format=%H", "--", path])
            .expect("full Git history is required; fetch-depth must be 0");
        let log = std::str::from_utf8(&history).unwrap();
        let commits: Vec<&str> = log.lines().collect();
        let mut valid_this_file = 0_usize;
        for pair in commits.windows(2) {
            if valid_this_file == 4 { break; }
            let older = pair[1];
            let newer = pair[0];
            let old_ref = format!("{older}:{path}");
            let new_ref = format!("{newer}:{path}");
            let (Some(old), Some(target)) = (
                git(&["show", &old_ref]),
                git(&["show", &new_ref]),
            ) else { continue };
            if old.len() < 64 || target.len() < 64 || old == target
                || old.len() > MAX_TEXT_BYTES || target.len() > MAX_TEXT_BYTES
                || std::str::from_utf8(&old).is_err()
                || std::str::from_utf8(&target).is_err()
            {
                continue;
            }

            let base_id = RevisionId::new();
            let target_id = RevisionId::new();
            let base = Some((base_id, old.as_slice()));
            let mut encoded_timings = Vec::with_capacity(repetitions);
            let mut decoded_timings = Vec::with_capacity(repetitions);
            let mut frame_bytes = None;
            let mut chosen_mode = None;
            for _ in 0..repetitions {
                let start = Instant::now();
                let frame = match mode.as_str() {
                    "full" => encode(target_id, &target, None).unwrap().bytes,
                    "splice" => {
                        let patch = splice(&old, &target).unwrap();
                        let compressed = zstd::bulk::compress(&patch, LEVEL).unwrap();
                        to_header(
                            TextDeltaMode::SpliceZstd, base,
                            target_id, &target, compressed
                        ).unwrap()
                    }
                    "dictionary" => {
                        let mut compressor = zstd::bulk::Compressor::with_dictionary(
                            LEVEL, &old
                        ).unwrap();
                        let compressed = compressor.compress(&target).unwrap();
                        to_header(
                            TextDeltaMode::DictionaryZstd, base,
                            target_id, &target, compressed
                        ).unwrap()
                    }
                    "selected" => encode(target_id, &target, base).unwrap().bytes,
                    _ => unreachable!(),
                };
                let elapsed_encode = start.elapsed().as_nanos() as u64;
                let start_decode = Instant::now();
                let restored = decode(
                    &frame, target_id,
                    if mode == "full" { None } else { base }
                ).unwrap();
                let elapsed_decode = start_decode.elapsed().as_nanos() as u64;
                assert_eq!(restored, target, "a codec mode failed byte-for-byte");
                let actual = match frame[9] {
                    0 => "full", 1 => "splice", 2 => "dictionary",
                    _ => panic!("unknown experimental codec mode"),
                };
                if let Some(first) = frame_bytes {
                    assert_eq!(first, frame.len(), "nondeterministic frame size");
                }
                frame_bytes = Some(frame.len());
                chosen_mode = Some(actual);
                encoded_timings.push(elapsed_encode);
                decoded_timings.push(elapsed_decode);
                std::hint::black_box(restored);
            }
            encoded_timings.sort_unstable();
            decoded_timings.sort_unstable();
            // Only paths/commit ids, lengths, digests and timing are emitted;
            // never write the contents of user or repository documents.
            println!(
                "DELTA_TEXT_RUST_MODE_JSON {}",
                serde_json::json!({
                    "schema": "akula-delta-text-04-real-git-v1",
                    "claim": "isolated-real-git-versions-not-production-sync",
                    "mode": mode.as_str(),
                    "actual_frame_mode": chosen_mode.unwrap(),
                    "path": path,
                    "from_commit": &older[..12],
                    "to_commit": &newer[..12],
                    "base_blake3": blake3::hash(&old).to_hex().to_string(),
                    "target_blake3": blake3::hash(&target).to_hex().to_string(),
                    "base_bytes": old.len(),
                    "target_bytes": target.len(),
                    "wire_bytes": frame_bytes.unwrap(),
                    "encode_median_ns": encoded_timings[repetitions / 2],
                    "decode_median_ns": decoded_timings[repetitions / 2],
                    "repetitions": repetitions,
                })
            );
            total += 1;
            valid_this_file += 1;
        }
        if valid_this_file > 0 { visited_paths += 1; }
    }
    assert!(total >= 9 && visited_paths >= 3,
        "requires complete real Git history with >=9 valid pairs across >=3 paths");
    println!("DELTA_TEXT_RUST_MODE_DONE mode={mode} pairs={total} paths={visited_paths}");
}
