#!/usr/bin/env python3
"""Require complete, pixel-validated Vello adjacent batching A/B measurements."""

from __future__ import annotations

import argparse
import re
import statistics
from pathlib import Path

ROW = re.compile(
    r"VELLO_ADJACENT_AB case=(\w+) strips=(\d+) full_every=(\d+) "
    r"mode=(\w+) workload=(\w+) frames=(\d+) rects_max=(\d+) "
    r"total_p50_us=([\d.]+) total_p95_us=([\d.]+) "
    r"reset_p50_us=([\d.]+) replay_p50_us=([\d.]+) "
    r"flush_p50_us=([\d.]+) raster_p50_us=([\d.]+) "
    r"total_sum_us=([\d.]+) reset_sum_us=([\d.]+) "
    r"replay_sum_us=([\d.]+) flush_sum_us=([\d.]+) "
    r"raster_sum_us=([\d.]+)"
)
CASES = ("text_1x", "text_175", "cover_175")
STRIPS = (2, 4, 8, 16)
EVERY = (8, 3)
MODES = ("serial", "parallel2", "coalesced0", "coalesced2")
WORKLOADS = ("scroll", "full")
EXPECTED = {
    (case, strips, interval, mode, workload)
    for case in CASES
    for strips in STRIPS
    for interval in EVERY
    for mode in MODES
    for workload in WORKLOADS
}


def parse(file: Path) -> dict[tuple, dict]:
    log = file.read_text(encoding="utf-8")
    if "test result: ok." not in log:
        raise ValueError(f"{file}: test not confirmed successful")
    values = {}
    for match in ROW.finditer(log):
        (
            case, strips, interval, mode, workload, frames, rects,
            p50, p95, reset_p50, replay_p50, flush_p50, raster_p50,
            total, reset, replay, flush, raster,
        ) = match.groups()
        key = (case, int(strips), int(interval), mode, workload)
        if key in values:
            raise ValueError(f"{file}: duplicate row {key}")
        if int(frames) < 4:
            raise ValueError(f"{file}: too few frames {key}")
        if workload == "full" and int(rects) != (1 if mode.startswith("coalesced") else int(strips)):
            raise ValueError(f"{file}: wrong number of full-frame raster passes {key}, {rects}")
        sums = [float(n) for n in (reset, replay, flush, raster)]
        if min(sums) < 0 or sum(sums) > float(total) + 5:
            raise ValueError(f"{file}: inconsistent non-overlapping stage sums {key}")
        values[key] = {
            "total": float(total),
            "reset": float(reset),
            "replay": float(replay),
            "flush": float(flush),
            "raster": float(raster),
            "p50": float(p50),
            "p95": float(p95),
        }
    if set(values) != EXPECTED:
        missing = EXPECTED - set(values)
        extra = set(values) - EXPECTED
        raise ValueError(
            f"{file}: {len(missing)} missing and {len(extra)} extra records: "
            f"{sorted(missing)[:15]}, {sorted(extra)[:15]}"
        )
    return values


def report(trials: list[dict]) -> str:
    lines = [
        "# Akula Vello exact-coverage adjacent batching — release-mode A/B",
        "",
        "Three independent Linux release processes. Four fixed/adjacent modes "
        "render identical Akula scenes and check byte-for-byte framebuffer "
        "parity on all 64 frames per scenario, plus <=1/255 versus a fresh "
        "independent full redraw at 6 checkpoints. No production code path changed.",
        "",
        "Each fragmented full frame consists of 2/4/8/16 touching strips. "
        "Coalescing must reduce these to ONE region without adding raster pixels. "
        "Overlapping or non-adjacent damage is never merged.",
        "",
        "| Strips | Scene | Full every | Baseline 0 | Coalesced 0 | Baseline 2 | Coalesced 2 | Best coalesced gain vs best baseline (median, range) |",
        "|---:|---|---:|---:|---:|---:|---:|---:|",
    ]
    distribution = {n: [] for n in STRIPS}
    for strips in STRIPS:
        for case in CASES:
            for interval in EVERY:
                times = [
                    {
                        mode: sum(trial[(case, strips, interval, mode, w)]["total"] for w in WORKLOADS)
                        for mode in MODES
                    }
                    for trial in trials
                ]
                medians = {
                    mode: statistics.median(entry[mode] for entry in times)
                    for mode in MODES
                }
                gains = [
                    (min(t["serial"], t["parallel2"]) - min(t["coalesced0"], t["coalesced2"]))
                    / min(t["serial"], t["parallel2"])
                    * 100.0
                    for t in times
                ]
                distribution[strips].extend(gains)
                lines.append(
                    f"| {strips} | {case} | {interval} "
                    f"| {medians['serial'] / 1000:.2f} ms "
                    f"| {medians['coalesced0'] / 1000:.2f} ms "
                    f"| {medians['parallel2'] / 1000:.2f} ms "
                    f"| {medians['coalesced2'] / 1000:.2f} ms "
                    f"| {statistics.median(gains):+.1f}% "
                    f"({min(gains):+.1f}…{max(gains):+.1f}%) |"
                )
    lines += [
        "",
        "## Cost of repeated resets / replay / flush",
        "",
        "Stage totals below aggregate only sampled full redraws; they are "
        "**not** estimates of entire application FPS. The reset bucket includes "
        "RenderContext reset and worker broadcast/barrier but does not isolate "
        "the barrier itself. Results shown are median over the three processes.",
        "",
        "| Strips | Mode | Reset total | Scene replay total | Flush total | Raster total |",
        "|---:|---|---:|---:|---:|---:|",
    ]
    for strips in STRIPS:
        for mode in MODES:
            stages = {}
            for stage in ("reset", "replay", "flush", "raster"):
                per_trial = [
                    sum(
                        trial[(case, strips, interval, mode, "full")][stage]
                        for case in CASES
                        for interval in EVERY
                    )
                    for trial in trials
                ]
                stages[stage] = statistics.median(per_trial) / 1000.0
            lines.append(
                f"| {strips} | {mode} | {stages['reset']:.2f} ms "
                f"| {stages['replay']:.2f} ms | {stages['flush']:.2f} ms "
                f"| {stages['raster']:.2f} ms |"
            )
    lines += [
        "",
        "## Observed crossover",
        "",
        "| Strips | Coalescing beats best fixed mode | Trials |",
        "|---:|---:|---:|",
    ]
    for strips, gains in distribution.items():
        lines.append(f"| {strips} | {sum(x > 0 for x in gains)} | {len(gains)} |")
    lines += [
        "",
        "A positive number is evidence about **adjacent full-edge strips only**. "
        "It does not imply that disjoint rectangles should be merged via their "
        "bounding boxes. Independently validate sparse scenes, overlapping "
        "alpha, glyph accuracy, memory and cross-platform behavior before "
        "considering a production change.",
        "",
    ]
    return "\n".join(lines)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("logs", nargs=3, type=Path)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    result = report([parse(path) for path in args.logs])
    args.output.write_text(result, encoding="utf-8")
    print(result)


if __name__ == "__main__":
    main()
