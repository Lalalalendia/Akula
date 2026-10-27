#!/usr/bin/env python3
"""Non-gating same-runner A/B of real-frame damage stages, not UI FPS."""
import argparse
import math
from pathlib import Path
import statistics
import subprocess

CASES = ("standard_1x", "fractional_1_75x")
FIELDS = ("record_p50_us", "record_p95_us", "detect_p50_us", "detect_p95_us")
ROUNDS = 4


def sample(binary, label, index, directory):
    command = [binary, "benchmark_damage_frame_stages", "--ignored", "--nocapture", "--test-threads=1"]
    process = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, check=False)
    (directory / f"{index:02d}-{label}.log").write_text(process.stdout)
    if process.returncode:
        raise RuntimeError(f"{label} run {index} failed (exit {process.returncode})")
    results = {}
    parity = set()
    for line in process.stdout.splitlines():
        if "DAMAGE_PROFILE case=" in line:
            values = dict(part.split("=", 1) for part in line.split("DAMAGE_PROFILE ", 1)[1].split() if "=" in part)
            results[values["case"]] = values
        if "DAMAGE_PARITY case=" in line:
            values = dict(part.split("=", 1) for part in line.split("DAMAGE_PARITY ", 1)[1].split() if "=" in part)
            if values.get("pixel_differences") == "0" and values.get("checked") == "4":
                parity.add(values["case"])
    if set(results) != set(CASES) or parity != set(CASES):
        raise RuntimeError(f"Missing profile/parity evidence for {label} run {index}: {results.keys()}, {parity}")
    return results


def percentile(values, fraction):
    ordered = sorted(values)
    return ordered[max(0, math.ceil(len(ordered) * fraction) - 1)]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("baseline")
    parser.add_argument("candidate")
    parser.add_argument("out_dir")
    args = parser.parse_args()
    directory = Path(args.out_dir)
    directory.mkdir(parents=True, exist_ok=True)
    binaries = {"base": args.baseline, "candidate": args.candidate}
    results = {"base": [], "candidate": []}
    # An equal-cost warmup keeps the first cold setup out of the paired distribution.
    for mode in ("base", "candidate"):
        sample(binaries[mode], f"{mode}-warmup", 0, directory)
    count = 0
    for round_index in range(ROUNDS):
        for mode in ("base", "candidate", "candidate", "base"):
            count += 1
            results[mode].append(sample(binaries[mode], mode, count, directory))
    report = [
        "# Argon 8.1 damage prehash: Linux A/B (diagnostic)",
        "",
        "Four ABBA rounds, 8 samples per binary, 48 real scrolling/pointer frames per case.",
        "Two separately compiled revisions, same runner, debug/test instrumentation.",
        "All sampled runs have 4/4 partial-versus-full pixel parity checks per case.",
        "Time is microseconds. Smaller is better. This is NOT end-to-end latency",
        "or a release-mode speedup; n=8 p95 is the maximum of eight samples.",
        "",
        "| Case | Stage | Base p50 / p95 | Candidate p50 / p95 | Median ratio (base/candidate) |",
        "|---|---|---:|---:|---:|",
    ]
    for case in CASES:
        for field in FIELDS:
            baseline = [float(x[case][field]) for x in results["base"]]
            candidate = [float(x[case][field]) for x in results["candidate"]]
            b50, b95 = statistics.median(baseline), percentile(baseline, .95)
            c50, c95 = statistics.median(candidate), percentile(candidate, .95)
            ratio = b50 / c50 if c50 > 0 else float("nan")
            report.append(f"| {case} | {field} | {b50:.2f} / {b95:.2f} | {c50:.2f} / {c95:.2f} | {ratio:.3f}x |")
    text = "\n".join(report) + "\n"
    (directory / "summary.md").write_text(text)
    print(text)


if __name__ == "__main__":
    main()
