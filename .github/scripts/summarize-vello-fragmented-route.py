#!/usr/bin/env python3
"""Check 8-strip fragmentation routing against fixed 0/2-worker alternatives."""

import argparse
import re
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


def summarize(log: str) -> str:
    data = {}
    for m in RE.finditer(log):
        case, interval, mode, area, frames, strips, per_rect, frame, p50, p95, total = m.groups()
        key = case, int(interval), mode, area
        if key in data:
            raise ValueError(f"duplicate measurement: {key}")
        if int(frames) < 4 or int(strips) != 8 or int(per_rect) > 2 or int(frame) < 6:
            raise ValueError(f"workload or switching invariants failed: {key} ({m.group()})")
        data[key] = (int(frames), float(p50), float(p95), float(total))

    expected = {
        (c, n, mode, segment)
        for c in CASES for n in EVERY for mode in MODES for segment in CLASSES
    }
    if set(data) != expected:
        raise ValueError(
            f"missing {sorted(expected - set(data))}; unexpected {sorted(set(data) - expected)}"
        )

    lines = [
        "# Vello 8-strip full redraw — per-rect vs per-frame routing",
        "",
        "Every full redraw is split into eight disjoint horizontal strips.",
        "A full frame remains the same total physical pixels, but individual "
        "rectangles fall below the 350k experimental routing threshold.",
        "All four variants must be byte-identical across 64 editor frames, "
        "and agree with the independent non-fragmented full reference to 1 LSB.",
        "",
        "| Scene | Full every | Serial total | 2-worker total | Per-rect route | Per-frame budget | Gain frame vs per-rect |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]

    gains = []
    for case in CASES:
        for interval in EVERY:
            total = {
                mode: sum(data[(case, interval, mode, segment)][3] for segment in CLASSES)
                for mode in MODES
            }
            gain = (total["per_rect"] - total["frame_budget"]) / total["per_rect"] * 100.0
            gains.append(gain)
            lines.append(
                f"| {case} | {interval} | {total['serial']/1000:.2f} ms "
                f"| {total['parallel2']/1000:.2f} ms "
                f"| {total['per_rect']/1000:.2f} ms "
                f"| {total['frame_budget']/1000:.2f} ms | {gain:+.1f}% |"
            )

    lines += [
        "",
        f"Frame-budget routing won {sum(gain > 0 for gain in gains)} of {len(gains)} "
        "mixed scenarios on this single Linux run.",
        "Results show candidate workload-classification behavior, not a "
        "general product policy. Repeat and inspect p95/memory/platform "
        "variation before promoting routing to production.",
        "",
    ]
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("log", type=Path)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    report = summarize(args.log.read_text(encoding="utf-8"))
    args.output.write_text(report, encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
