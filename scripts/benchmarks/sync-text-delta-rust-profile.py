#!/usr/bin/env python3
"""DELTA-TEXT-04: inspect *actual Rust* Zstd frame sizes, time and process RSS.

This is a reproducible local experiment, NEVER a sync sender or File Gate.
The same public adjacent Git revisions as DELTA-TEXT-02 are read by the Rust
ignored test. Full/Splice/Raw-Dictionary and policy-selected modes are run in
separate processes to measure per-process CPU and peak RSS via Linux wait4().
Timing excludes Git subprocesses; no document content is written to output.
"""
from __future__ import annotations

import argparse
import csv
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import tempfile
import time

MODES = ("full", "splice", "dictionary", "selected")
PREFIX = "DELTA_TEXT_RUST_MODE_JSON "
CLAIM = "isolated-real-git-versions-not-production-sync"


def ensure(condition: bool, explanation: str) -> None:
    if not condition:
        raise AssertionError(explanation)


def select_binary(repo: Path) -> Path:
    """Compile once. Choose the exact Cargo lib-test executable, not a stale file."""
    cmd = [
        "cargo", "test", "--manifest-path", "modules/text-delta-codec/Cargo.toml",
        "--lib", "--no-run", "--message-format=json",
    ]
    proc = subprocess.run(
        cmd, cwd=repo, text=True, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, check=False, timeout=1200,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"Rust test build failed:\n{proc.stderr[-4500:]}")
    candidates = []
    for line in proc.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            message.get("reason") == "compiler-artifact"
            and message.get("target", {}).get("name") == "akula_text_delta_codec"
            and message.get("profile", {}).get("test")
            and message.get("executable")
        ):
            candidates.append(Path(message["executable"]))
    if len(candidates) != 1 or not candidates[0].is_file():
        raise RuntimeError(f"Expected exactly one fresh standalone codec lib test binary; got {candidates}")
    return candidates[0]


def run_mode(repo: Path, executable: Path, mode: str, reps: int):
    """Wait4 accounts for this Rust test process only, not Cargo or child git."""
    if not hasattr(os, "wait4") or sys.platform != "linux":
        raise RuntimeError("Process CPU/RSS measurements require Linux wait4()")
    env = dict(os.environ)
    env["AKULA_DELTA_RUST_BENCH_MODE"] = mode
    env["AKULA_DELTA_RUST_BENCH_REPS"] = str(reps)
    args = [
        str(executable), "codec::tests::real_git_markdown_pairs_mode_report",
        "--ignored", "--nocapture", "--test-threads=1",
    ]
    with tempfile.TemporaryFile(mode="w+t", encoding="utf-8") as output:
        start = time.perf_counter()
        process = subprocess.Popen(
            args, cwd=repo, stdout=output, stderr=subprocess.STDOUT,
            env=env, close_fds=True,
        )
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        elapsed = time.perf_counter() - start
        output.seek(0)
        log = output.read()
    if process.returncode != 0:
        raise RuntimeError(f"Rust mode {mode} failed (rc={process.returncode}):\n{log[-5000:]}")
    entries = []
    for line in log.splitlines():
        if PREFIX in line:
            value = json.loads(line.split(PREFIX, 1)[1])
            ensure(value["mode"] == mode, "mode must match Rust process")
            ensure(value["claim"] == CLAIM, "untrusted benchmark claim")
            entries.append(value)
    ensure(len(entries) >= 9, f"too few real Git pairs in mode {mode}: {len(entries)}")
    ensure(len({r["path"] for r in entries}) >= 3, "benchmark corpus too narrow")
    for r in entries:
        ensure(r["wire_bytes"] >= 120, "missing Rust wire envelope")
        ensure(r["base_bytes"] >= 64 and r["target_bytes"] >= 64, "invalid Git fixture")
        ensure(r["encode_median_ns"] > 0 and r["decode_median_ns"] > 0, "missing Rust timings")
        ensure(r["repetitions"] == reps, "inconsistent repetition count")
        if mode != "selected":
            ensure(r["actual_frame_mode"] == mode, "mode did not really use selected encoder")
    profile = {
        "mode": mode,
        "process_cpu_s": round(usage.ru_utime + usage.ru_stime, 6),
        "process_wall_s": round(elapsed, 6),
        # Linux ru_maxrss is KiB, and includes all Rust test runtime + fixtures.
        "rust_test_process_peak_rss_kib": usage.ru_maxrss,
    }
    return entries, profile


def pair_id(r):
    return (r["path"], r["from_commit"], r["to_commit"],
            r["base_blake3"], r["target_blake3"])


def compare(data):
    """Check mode pairing, fallback and identical *complete frame* accounting."""
    ensure(set(data) == set(MODES), "all four independent codecs are required")
    by_mode = {}
    for mode, records in data.items():
        pairs = {pair_id(r): r for r in records}
        ensure(len(pairs) == len(records), f"duplicate Git pair in {mode}")
        by_mode[mode] = pairs
    ids = set(by_mode["full"])
    ensure(ids, "no real history found")
    for mode in MODES:
        ensure(set(by_mode[mode]) == ids, f"different observed Git pairs in {mode}")
    paired = []
    for key in sorted(ids):
        full = by_mode["full"][key]
        selected = by_mode["selected"][key]
        for mode in MODES:
            r = by_mode[mode][key]
            ensure(r["base_bytes"] == full["base_bytes"], "different base inputs")
            ensure(r["target_bytes"] == full["target_bytes"], "different target inputs")
        ensure(selected["wire_bytes"] <= full["wire_bytes"], "adaptive policy worse than full")
        if selected["actual_frame_mode"] != "full":
            saving = full["wire_bytes"] - selected["wire_bytes"]
            ensure(saving >= 128, "dependent frame saves less than 128 bytes")
            ensure(saving * 100 >= 15 * full["wire_bytes"], "dependent frame saves below 15%")
        # The Rust raw-dictionary codec is NOT the CLI --patch-from format.
        preferred = min(
            MODES[:3],
            key=lambda m: by_mode[m][key]["wire_bytes"],
        )
        ensure(selected["actual_frame_mode"] in MODES[:3], "unexpected policy mode")
        line = {
            "public_path": full["path"],
            "base_commit": full["from_commit"],
            "target_commit": full["to_commit"],
            "base_blake3": full["base_blake3"],
            "target_blake3": full["target_blake3"],
            "base_raw_bytes": full["base_bytes"],
            "target_raw_bytes": full["target_bytes"],
            "full_wire_bytes": full["wire_bytes"],
            "splice_wire_bytes": by_mode["splice"][key]["wire_bytes"],
            "dictionary_wire_bytes": by_mode["dictionary"][key]["wire_bytes"],
            "selected_wire_bytes": selected["wire_bytes"],
            "selected_mode": selected["actual_frame_mode"],
            "smallest_forced_candidate": preferred,
            "selected_saving_vs_full_bytes": full["wire_bytes"] - selected["wire_bytes"],
            # HYPOTHETICAL: V1 still obliges sending full immutable raw blob.
            "mandatory_v1_plus_full_sidecar_bytes": full["target_bytes"] + full["wire_bytes"],
            "mandatory_v1_plus_selected_sidecar_bytes": full["target_bytes"] + selected["wire_bytes"],
            # COLD receiver should simply select full (no forced old-base fetch).
            "cold_receiver_full_fallback_bytes": full["wire_bytes"],
            # COUNTERFACTUAL bad choice: fetch raw old base ONLY for dependent frame.
            "naive_cold_fetch_base_plus_selected_bytes": (
                full["wire_bytes"] if selected["actual_frame_mode"] == "full"
                else full["base_bytes"] + selected["wire_bytes"]
            ),
            "claim": CLAIM,
        }
        paired.append(line)
    return paired


def aggregate(data, profiling, pairs, reps):
    selected_sum = sum(x["selected_wire_bytes"] for x in pairs)
    full_sum = sum(x["full_wire_bytes"] for x in pairs)
    raw_target_sum = sum(x["target_raw_bytes"] for x in pairs)
    rows = []
    for mode in MODES:
        records = data[mode]
        wire_sum = sum(r["wire_bytes"] for r in records)
        rows.append({
            "mode": mode,
            "real_git_pairs": len(records),
            "repetitions_per_pair": reps,
            "wire_total_bytes": wire_sum,
            "full_zstd_wire_bytes": full_sum,
            "delta_sidecar_saving_percent_vs_full": round(
                100 * (full_sum - wire_sum) / full_sum, 3
            ),
            "raw_target_sum_bytes": raw_target_sum,
            "hypothetical_required_v1_plus_sidecar_total_bytes": raw_target_sum + wire_sum,
            "rust_pair_encode_median_ms": round(
                statistics.median(r["encode_median_ns"] for r in records) / 1_000_000, 4),
            "rust_pair_decode_median_ms": round(
                statistics.median(r["decode_median_ns"] for r in records) / 1_000_000, 4),
            "process_cpu_s_including_git_io_dispatch_not_git_children": profiling[mode]["process_cpu_s"],
            "process_wall_s_including_history_git": profiling[mode]["process_wall_s"],
            "rust_test_process_peak_rss_kib": profiling[mode]["rust_test_process_peak_rss_kib"],
            "claim": CLAIM,
        })
    return rows


def export_report(out: Path, rows, pairs, reps):
    by_mode = {r["mode"]: r for r in rows}
    full = by_mode["full"]
    selected = by_mode["selected"]
    n = len(pairs)
    savings = 100 * (
        full["wire_total_bytes"] - selected["wire_total_bytes"]
    ) / full["wire_total_bytes"]
    mandatory_full = full["hypothetical_required_v1_plus_sidecar_total_bytes"]
    mandatory_selected = selected["hypothetical_required_v1_plus_sidecar_total_bytes"]
    lines = [
        "# DELTA-TEXT-04 — bounded Rust Zstd actual historical Git revisions",
        "",
        "**NOT a live Akula transfer or proof of lower S3 traffic.**",
        f"Corpus: {n} actual Git adjacent-version pairs, {len(set(p['public_path'] for p in pairs))} public Markdown paths;",
        f"{reps} repetitions per pair and mode, Linux isolated Rust test processes.",
        "The selected policy chooses FullZstd unless an exact-base delta saves",
        ">=128 bytes AND >=15% of the complete versioned wire envelope.",
        "",
        "| Rust mode | Full wire bytes | Mode wire bytes | Sidecar savings % | Median encode ms/pair | Median decode ms/pair | Rust test CPU s | Peak RSS KiB |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for m in MODES:
        r = by_mode[m]
        lines.append(
            f"| {m} | {r['full_zstd_wire_bytes']} | {r['wire_total_bytes']} | "
            f"{r['delta_sidecar_saving_percent_vs_full']:.3f} | "
            f"{r['rust_pair_encode_median_ms']:.4f} | "
            f"{r['rust_pair_decode_median_ms']:.4f} | "
            f"{r['process_cpu_s_including_git_io_dispatch_not_git_children']:.4f} | "
            f"{r['rust_test_process_peak_rss_kib']} |"
        )
    lines += [
        "",
        "## Accounting of hypothetical mandatory V1 raw object + alternate sidecar",
        "",
        f"- Sidecar full Zstd total: {full['wire_total_bytes']} bytes.",
        f"- Policy-selected sidecar total: {selected['wire_total_bytes']} bytes.",
        f"- Reduction of this OPTIONAL sidecar only: {savings:.3f}%.",
        f"- Hypothetical FULL V1 raw objects plus full sidecars: {mandatory_full} bytes.",
        f"- Hypothetical FULL V1 raw objects plus selected sidecars: {mandatory_selected} bytes.",
        "- These sums are accounting models, NOT measured network PUT/GET of live sync.",
        "- On a receiver cache miss, use FullZstd; do NOT fetch untrusted old bytes",
        "  solely to apply a delta. Actual S3 cache/network costs need a separate test.",
        "- Full V1 objects are still immutable and mandatory in the current stack.",
        "",
        "## Timing and RAM limitations",
        "",
        "- Encode/decode medians are std::time::Instant wall time inside Rust tests.",
        "- Process CPU is Linux wait4() user+system time for the Rust test process;",
        "  it includes Rust test setup but excludes child Git subprocess CPU.",
        "- ru_maxrss is process-wide peak RSS KiB, includes Rust test harness and",
        "  Git fixture buffers. It does NOT measure per-frame allocation or mobile RAM.",
        "- Every mode runs in a separate process, avoiding monotonic peak mixing.",
        "- Timings are on one GitHub Actions virtual machine. Re-run for noise;",
        "  do not claim stable CPU speedups, energy, or Rust codec hardware limits.",
        "- Original PR #276 used CLI Zstd --patch-from, not this raw-dictionary",
        "  Rust codec. No historical 89.87% ratio is imputed to these results.",
        "",
        "## Security / implementation limits",
        "",
        "No sync writer, File Gate privilege, V1/V2 wire or production protocol changed.",
        "New Rust text frames remain isolated, opt-in and versioned. No user data",
        "is included in any exported CSV/JSON; public path, hashes and commit ids only.",
    ]
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_csv(path: Path, rows) -> None:
    if not rows:
        raise ValueError("cannot save empty benchmark")
    with path.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def test_accounting() -> None:
    common = {
        "schema": "akula-delta-text-04-real-git-v1",
        "claim": CLAIM, "path": "demo.md", "from_commit": "0" * 12,
        "to_commit": "1" * 12, "base_blake3": "a" * 64,
        "target_blake3": "b" * 64, "base_bytes": 500,
        "target_bytes": 300, "encode_median_ns": 100,
        "decode_median_ns": 200, "repetitions": 3,
    }
    specs = [("full", "full", 300), ("splice", "splice", 90),
             ("dictionary", "dictionary", 180), ("selected", "splice", 90)]
    data = {m: [dict(common, mode=m, actual_frame_mode=actual, wire_bytes=size)]
            for m, actual, size in specs}
    rows = compare(data)
    ensure(len(rows) == 1 and rows[0]["selected_saving_vs_full_bytes"] == 210,
           "wrong V1 sidecar savings calculation")
    ensure(rows[0]["mandatory_v1_plus_selected_sidecar_bytes"] == 390,
           "must add V1 mandatory raw copy")
    ensure(rows[0]["naive_cold_fetch_base_plus_selected_bytes"] == 590,
           "cold wrong-base fetch must cost base bytes")
    bad = dict(data)
    bad["selected"] = [dict(data["selected"][0], wire_bytes=280)]
    try:
        compare(bad)
    except AssertionError:
        pass
    else:
        raise AssertionError("must reject selected delta below required 15%+128B")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path("."))
    parser.add_argument("--output", type=Path, default=Path("results/text-delta-04"))
    parser.add_argument("--repetitions", type=int, default=7)
    parser.add_argument("--self-test-only", action="store_true")
    args = parser.parse_args()
    ensure(1 <= args.repetitions <= 25, "require 1..25 repetitions")
    test_accounting()
    if args.self_test_only:
        print("DELTA_TEXT_RUST_AB_SELF_TEST_PASS", flush=True)
        return
    repo = args.repo.resolve()
    ensure((repo / "aquilum-app" / "Cargo.toml").is_file(), "wrong repository root")
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = select_binary(repo)
    by_mode = {}
    profiles = {}
    for mode in MODES:
        by_mode[mode], profiles[mode] = run_mode(repo, binary, mode, args.repetitions)
        print(f"RUST_MODE={mode}: {len(by_mode[mode])} pairs; "
              f"CPU={profiles[mode]['process_cpu_s']} s; "
              f"RSS={profiles[mode]['rust_test_process_peak_rss_kib']} KiB", flush=True)
    pairs = compare(by_mode)
    summary = aggregate(by_mode, profiles, pairs, args.repetitions)
    write_csv(out / "real-git-pairs.csv", pairs)
    write_csv(out / "mode-summary.csv", summary)
    export_report(out, summary, pairs, args.repetitions)
    (out / "report.json").write_text(json.dumps({
        "schema": "akula-delta-text-04-ab-v1",
        "claim": CLAIM,
        "git_pair_count": len(pairs),
        "source_paths": sorted({p["public_path"] for p in pairs}),
        "mode_summary": summary, "pairs": pairs,
    }, indent=2) + "\n", encoding="utf-8")
    print(f"DELTA_TEXT_RUST_AB_COMPLETE pairs={len(pairs)} "
          f"modes={len(summary)} corpus=real_git", flush=True)


if __name__ == "__main__":
    main()
