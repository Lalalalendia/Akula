#!/usr/bin/env python3
"""Validate and summarize test-only persistent Vello 0/2-worker routing."""

import argparse
import re
from pathlib import Path

PATTERN = re.compile(
    r"VELLO_DUAL_ROUTE case=(\w+) full_every=(\d+) mode=(\w+) "
    r"workload=(\w+) frames=(\d+) route_switches=(\d+) "
    r"p50_us=([\d.]+) p95_us=([\d.]+) total_us=([\d.]+)"
)
CASES = ("text_1x", "text_175", "cover_175")
FREQUENCIES = (8, 3)
MODES = ("serial", "parallel2", "routed")
WORKLOADS = ("scroll", "full")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("log", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    found = {}
    for match in PATTERN.finditer(args.log.read_text(encoding="utf-8")):
        case, every, mode, workload, frames, switches, p50, p95, total = match.groups()
        key = (case, int(every), mode, workload)
        if key in found:
            raise ValueError(f"duplicate route measurement: {key}")
        if int(frames) < 4 or int(switches) < 6:
            raise ValueError(f"insufficient exercise of route: {key}, {frames} frames, {switches} switches")
        found[key] = (int(frames), float(p50), float(p95), float(total))

    expected = {
        (case, every, mode, workload)
        for case in CASES for every in FREQUENCIES
        for mode in MODES for workload in WORKLOADS
    }
    if set(found) != expected:
        raise ValueError(f"missing: {sorted(expected - set(found))}; unexpected: {sorted(set(found) - expected)}")

    out = [
        "# Vello dual-dispatch proof — Linux release",
        "",
        "Test-only persistent 0/2 worker contexts share one glyph/image resource cache.",
        "This benchmark alternates actual scroll strips and full-frame invalidations.",
        "Measured draw_frame() includes context selection, encoding and raster work;",
        "it is NOT a full application FPS measurement.",
        "",
        "| Scene | Full every | Workload | Serial p50 | 2-worker p50 | Routed p50 | Routed vs segment-best fixed (total) |",
        "|---|---:|---|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for every in FREQUENCIES:
            for workload in WORKLOADS:
                a, b, r = (
                    found[(case, every, mode, workload)]
                    for mode in MODES
                )
                best_fixed = min(a[3], b[3])
                gain = (best_fixed - r[3]) / best_fixed * 100.0
                out.append(
                    f"| {case} | {every} | {workload} "
                    f"| {a[1]:.1f} µs | {b[1]:.1f} µs | {r[1]:.1f} µs "
                    f"| {gain:+.1f}% |"
                )
    out += [
        "",
        "## Combined workload comparison (sum of sampled frame times)",
        "",
        "A fixed renderer must handle **both** scroll and full frames. "
        "Comparing the routed mode separately to the best fixed mode in each "
        "segment gives an impossible oracle baseline and understates its benefit.",
        "",
        "| Scene | Full every | Serial total | 2-worker total | Routed total | Gain vs best fixed |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for every in FREQUENCIES:
            totals = {
                mode: sum(found[(case, every, mode, segment)][3] for segment in WORKLOADS)
                for mode in MODES
            }
            baseline = min(totals["serial"], totals["parallel2"])
            gain = (baseline - totals["routed"]) / baseline * 100.0
            out.append(
                f"| {case} | {every} "
                f"| {totals['serial'] / 1000:.2f} ms "
                f"| {totals['parallel2'] / 1000:.2f} ms "
                f"| {totals['routed'] / 1000:.2f} ms "
                f"| {gain:+.1f}% |"
            )

    out += [
        "",
        "Totals sum actual sampled frame times; do not sum p50 medians.",
        "A positive gain is provisional evidence; repeated ABBA runs, "
        "tail-latency and memory analysis are required before product changes.",
        "",
    ]
    result = "\n".join(out)
    args.output.write_text(result, encoding="utf-8")
    print(result)


if __name__ == "__main__":
    main()
