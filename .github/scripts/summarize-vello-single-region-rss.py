#!/usr/bin/env python3
"""Validate independent release A/B trials with one Vello renderer per process."""
import argparse
from pathlib import Path
import re
from statistics import median

PATTERN = re.compile(
    r"VELLO_SINGLE_REGION_PROCESS_RSS mode=(\w+) frames=(\d+) shifts=(\d+) "
    r"full_frames=(\d+) small_frames=(\d+) switches=(\d+) "
    r"peak_kib=(\d+) p50_us=([\d.]+) p95_us=([\d.]+) total_us=([\d.]+)"
)
MODES = ("serial", "parallel2", "routed")
TRIALS = (1, 2, 3)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("folder", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    rows = {}
    for mode in MODES:
        for trial in TRIALS:
            path = args.folder / f"vello-single-region-{mode}-trial-{trial}.log"
            if not path.is_file():
                raise SystemExit(f"Missing independent process: {path}")
            matches = list(PATTERN.finditer(path.read_text(encoding="utf-8")))
            if len(matches) != 1:
                raise SystemExit(f"Expected one process metric in {path}, got {len(matches)}")
            got_mode, frames, shifts, full, small, switches, peak, p50, p95, total = matches[0].groups()
            if got_mode != mode or min(int(frames), int(shifts), int(full), int(small), int(peak)) <= 0:
                raise SystemExit(f"Bad mode/empty workload in {path}")
            if int(frames) < 45 or int(full) < 6 or int(small) < 30:
                raise SystemExit(f"Insufficient real UI coverage in {path}")
            if (mode == "routed" and int(switches) < 8) or (mode != "routed" and int(switches) != 0):
                raise SystemExit(f"Unexpected dispatcher switches for {path}: {switches}")
            rows[(mode, trial)] = {
                "rss": int(peak), "p50": float(p50), "p95": float(p95),
                "total": float(total), "frames": int(frames)
            }

    medians = {mode: {
        metric: median(rows[(mode, trial)][metric] for trial in TRIALS)
        for metric in ("rss", "p50", "p95", "total")
    } for mode in MODES}
    best_fixed = min(MODES[:2], key=lambda mode: medians[mode]["total"])
    gain = 100.0 * (medians[best_fixed]["total"] - medians["routed"]["total"]) / medians[best_fixed]["total"]
    overhead = medians["routed"]["rss"] - medians[best_fixed]["rss"]
    lines = [
        "# Akula Vello: single dirty region, separate-process A/B and peak RSS",
        "",
        "Linux release build; 3 independent test processes for each renderer.",
        "The reported VmHWM is whole **test process** high-water RSS, not a",
        "per-context allocation. CPU variance, font caches and other process",
        "memory contribute. Frame timings are actual draw_frame() costs",
        "on the same synthetic real-editor event sequence, not app FPS.",
        "",
        "| Mode | Median total sampled time | Median frame p50 | Median frame p95 | Median peak RSS | Peak RSS trial range |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for mode in MODES:
        metrics = medians[mode]
        peaks = [rows[(mode, trial)]["rss"] for trial in TRIALS]
        lines.append(
            f'| {mode} | {metrics["total"]/1000:.3f} ms | {metrics["p50"]:.1f} us '
            f'| {metrics["p95"]:.1f} us | {metrics["rss"]/1024:.2f} MiB '
            f'| {min(peaks)/1024:.2f}-{max(peaks)/1024:.2f} MiB |'
        )
    lines.extend([
        "",
        f"Routed total-time delta versus the better *fixed complete-run* median "
        f"({best_fixed}): **{gain:+.2f}%**.",
        f"Routed peak-process-RSS difference from that mode: **{overhead/1024:+.2f} MiB**.",
        "",
        "No threshold is inferred from 3 trials. Before enabling production by",
        "default, verify memory, p95 and aggregate gain on multiple machines",
        "and pass all native frame pixel-parity checks.",
        "",
    ])
    content = "\n".join(lines)
    args.output.write_text(content, encoding="utf-8")
    print(content)


if __name__ == "__main__":
    main()
