#!/usr/bin/env python3
"""Validate three independent 8-strip routing trials, including strict parity logs."""

import argparse
import re
import statistics
from pathlib import Path

RE = re.compile(
    r"VELLO_FRAGMENTED_ROUTE case=(\w+) full_every=(\d+) mode=(\w+) "
    r"workload=(\w+) frames=(\d+) strips=(\d+) per_rect_switches=(\d+) "
    r"frame_switches=(\d+) p50_us=([\d.]+) p95_us=([\d.]+) total_us=([\d.]+)"
)
CASES = ("text_1x", "text_175", "cover_175")
MODES = ("serial", "parallel2", "per_rect", "frame_budget")
EVERY = (8, 3)
CLASSES = ("scroll", "full")
EXPECTED = {
    (c, n, mode, segment)
    for c in CASES for n in EVERY for mode in MODES for segment in CLASSES
}


def parse_trial(file: Path) -> dict[tuple[str, int, str, str], tuple[int, float]]:
    data = {}
    text = file.read_text(encoding="utf-8")
    for m in RE.finditer(text):
        case, interval, mode, area, frames, strips, per_rect, frame, p50, p95, total = m.groups()
        key = case, int(interval), mode, area
        if key in data:
            raise ValueError(f"{file}: duplicate measurement {key}")
        if int(frames) < 4 or int(strips) != 8 or int(per_rect) > 2 or int(frame) < 6:
            raise ValueError(f"{file}: workload and switching invariants failed: {key}")
        data[key] = int(frames), float(total)
    if set(data) != EXPECTED:
        raise ValueError(
            f"{file}: missing {sorted(EXPECTED - set(data))}; unexpected {sorted(set(data) - EXPECTED)}"
        )
    return data


def summarize(trials: list[dict]) -> str:
    lines = [
        "# Vello fragmented redraw: three independent Linux release trials",
        "",
        "Every full redraw is divided into eight disjoint horizontal strips. "
        "All fixed/routed variants must remain byte-identical and pass the "
        "unsplit full reference at <=1/255 (enforced by each Rust test).",
        "All three process-level trials must finish successfully and report "
        "the complete 48-row benchmark matrix.",
        "",
        "| Scene | Full every | Serial total (median) | 2-worker (median) | Per-rect (median) | Frame-budget (median) | Frame-budget vs per-rect gain (median; range) |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    overall_gains = []
    for case in CASES:
        for interval in EVERY:
            per_trial = [
                {
                    mode: sum(trial[(case, interval, mode, area)][1] for area in CLASSES)
                    for mode in MODES
                }
                for trial in trials
            ]
            times = {
                mode: statistics.median(result[mode] for result in per_trial)
                for mode in MODES
            }
            gains = [
                (r["per_rect"] - r["frame_budget"]) / r["per_rect"] * 100.0
                for r in per_trial
            ]
            overall_gains.extend(gains)
            lines.append(
                f"| {case} | {interval} | {times['serial']/1000:.2f} ms "
                f"| {times['parallel2']/1000:.2f} ms "
                f"| {times['per_rect']/1000:.2f} ms "
                f"| {times['frame_budget']/1000:.2f} ms "
                f"| {statistics.median(gains):+.1f}% ({min(gains):+.1f}…{max(gains):+.1f}%) |"
            )
    wins = sum(x > 0 for x in overall_gains)
    lines += [
        "",
        f"Frame-budget beat rectangle-local routing in {wins}/{len(overall_gains)} "
        "complete scene/trial combinations. Sign and spread matter more than one "
        "medal; this is not a production policy.",
        "Reported time is the sum of actual sampled scroll+full frame times "
        "within each process, never an addition of p50 values.",
        "CPU, memory/RSS, diverse content and Windows/macOS evidence remain "
        "independent acceptance conditions.",
        "",
    ]
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("logs", nargs=3, type=Path)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    report = summarize([parse_trial(f) for f in args.logs])
    args.output.write_text(report, encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
