#!/usr/bin/env python3
"""Read exact-base/PR Rust path microbenchmark runs in same-runner ABBA order."""
import argparse
from pathlib import Path
import re
import statistics
import subprocess

CASES = ("same_exact", "same_nfc", "same_different", "strip_exact", "strip_nfc")
MARK = re.compile(r"PATHS_ABBA case=(\S+) samples=(\d+) p50_ns_per_256=([\d.]+) p95_ns_per_256=([\d.]+)")


def sample(binary, label, index, out):
    command = [str(binary), "benchmark_path_exact_identity", "--ignored", "--nocapture", "--test-threads=1"]
    p = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=False)
    (out / f"{index:02d}-{label}.log").write_text(p.stdout)
    if p.returncode != 0:
        raise RuntimeError(f"{label} run {index} exited {p.returncode}")
    rows = {}
    for line in p.stdout.splitlines():
        m = MARK.search(line)
        if m:
            key, n, p50, p95 = m.groups()
            if key in rows or int(n) != 16:
                raise RuntimeError(f"duplicate/incomplete sample: {key}")
            rows[key] = (float(p50), float(p95))
    if set(rows) != set(CASES):
        raise RuntimeError(f"{label} run {index} missing cases: {set(CASES) - set(rows)}")
    return rows


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    revisions = {"base": args.baseline, "candidate": args.candidate}
    data = {key: [] for key in revisions}
    for key in revisions:
        sample(revisions[key], f"{key}-warmup", 0, out)
    index = 0
    for _ in range(4):
        for key in ("base", "candidate", "candidate", "base"):
            index += 1
            data[key].append(sample(revisions[key], key, index, out))
    rows = [
        "# Argon 8.3 paths: paired Rust Release A/B",
        "",
        "Same Linux runner; 4 ABBA rounds (8 runs each revision), 16 timed samples",
        "per run and case. Both binaries use identical test source; compilation",
        "excluded from timings. Unit: nanoseconds per batch of 256 path comparisons.",
        "Values are median of each run's p50/p95. Smaller is faster.",
        "This is an isolated CPU diagnostic, not FileTree end-to-end latency.",
        "",
        "| Case | Base p50 | Candidate p50 | Base/Candidate | Base p95 | Candidate p95 |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        base = statistics.median(v[case][0] for v in data["base"])
        cand = statistics.median(v[case][0] for v in data["candidate"])
        b95 = statistics.median(v[case][1] for v in data["base"])
        c95 = statistics.median(v[case][1] for v in data["candidate"])
        if base <= 0 or cand <= 0:
            raise RuntimeError("invalid measurement")
        rows.append(f"| {case} | {base:.2f} | {cand:.2f} | {base/cand:.3f}x | {b95:.2f} | {c95:.2f} |")
    rows += ["", "No automatic speed threshold or claim of product performance."]
    report = "\n".join(rows) + "\n"
    (out / "summary.md").write_text(report)
    print(report)


if __name__ == "__main__":
    main()
