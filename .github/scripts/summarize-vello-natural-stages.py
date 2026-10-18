#!/usr/bin/env python3
"""Strict aggregate of native Akula 0/2/4-worker real-scroll stage profiles."""

import argparse
import statistics
from pathlib import Path

CASES = ("cover_1x", "cover_175", "plain_175")
WORKERS = (0, 2, 4)
EXPECTED = {(case, threads) for case in CASES for threads in WORKERS}
FLOATS = (
    "total_p50_us", "total_p95_us",
    "record_p50_us", "record_p95_us",
    "detect_p50_us", "detect_p95_us",
    "reset_p50_us", "reset_p95_us",
    "replay_p50_us", "replay_p95_us",
    "flush_p50_us", "flush_p95_us",
    "raster_p50_us", "raster_p95_us",
    "raster_area_p50_pct", "raster_area_p95_pct", "total_sum_us",
)
INTEGERS = ("frames", "max_rects", "multi_rect_frames", "eligible_adjacent_frames")


def load(path: Path) -> dict[tuple[str, int], dict]:
    content = path.read_text(encoding="utf-8")
    if "test result: ok." not in content:
        raise ValueError(f"{path}: real-scroll Rust test never succeeded")
    found = {}
    for line in content.splitlines():
        if "VELLO_NATURAL_STAGES case=" not in line:
            continue
        tokens = line.split("VELLO_NATURAL_STAGES ", 1)[1].split()
        parts = dict(token.split("=", 1) for token in tokens if "=" in token)
        case, workers = parts["case"], int(parts["workers"])
        key = case, workers
        if key in found:
            raise ValueError(f"{path}: duplicate profile row {key}")
        values = {name: float(parts[name]) for name in FLOATS}
        values.update({name: int(parts[name]) for name in INTEGERS})
        if not 25 <= values["frames"] <= 48:
            raise ValueError(f"{path}: missing actual scroll workload: {key}, {values['frames']}")
        if not (
            0 <= values["eligible_adjacent_frames"]
            <= values["multi_rect_frames"]
            <= values["frames"]
        ):
            raise ValueError(f"{path}: inconsistent region eligibility: {key}")
        if not 0 <= values["max_rects"] <= 10:
            raise ValueError(f"{path}: suspicious region count: {key}")
        if any(values[name] < 0 for name in FLOATS):
            raise ValueError(f"{path}: negative timing: {key}")
        found[key] = values
    if set(found) != EXPECTED:
        raise ValueError(
            f"{path}: missing={sorted(EXPECTED - set(found))}, extra={sorted(set(found) - EXPECTED)}"
        )
    # Region geometry and frame inclusion must be independent of worker count.
    for case in CASES:
        baseline = found[(case, 0)]
        for worker in (2, 4):
            alt = found[(case, worker)]
            for field in INTEGERS:
                if baseline[field] != alt[field]:
                    raise ValueError(f"{path}: worker {worker} changes {field} in {case}")
    return found


def summarize(trials: list[dict]) -> str:
    def median(case: str, workers: int, name: str) -> float:
        return statistics.median(data[(case, workers)][name] for data in trials)

    lines = [
        "# Akula Vello natural-scroll stage breakdown",
        "",
        "Three independent Linux release processes; 0, 2 and 4 fixed raster workers.",
        "No artificial damage fragmentation, guard bands, forced full frame or "
        "runtime routing. All worker modes must be **byte-identical** on every "
        "frame, with independent full-redraw comparisons <=1/255 at four checkpoints.",
        "Timings cover render stages only, not complete application UI latency.",
        "",
        "| Real scene | Workers | Scroll frames | Dirty rects max | Multi-rect frames | Exact-edge eligible | Total p50 | Total p95 | Reset p50 | Replay p50 | Flush p50 | Raster p50 |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for workers in WORKERS:
            values = {field: median(case, workers, field) for field in (*INTEGERS, *FLOATS)}
            lines.append(
                f"| {case} | {workers} | {int(values['frames'])} "
                f"| {int(values['max_rects'])} | {int(values['multi_rect_frames'])} "
                f"| {int(values['eligible_adjacent_frames'])} "
                f"| {values['total_p50_us']:.1f} µs | {values['total_p95_us']:.1f} µs "
                f"| {values['reset_p50_us']:.1f} µs | {values['replay_p50_us']:.1f} µs "
                f"| {values['flush_p50_us']:.1f} µs | {values['raster_p50_us']:.1f} µs |"
            )
    lines += [
        "",
        "## Natural workload suitability",
        "",
        "| Scene | Typical damage | Real full-edge coalescing opportunities |",
        "|---|---:|---:|",
    ]
    for case in CASES:
        samples = int(median(case, 0, "frames"))
        regions = int(median(case, 0, "max_rects"))
        eligible = int(median(case, 0, "eligible_adjacent_frames"))
        lines.append(f"| {case} | <= {regions} rectangles | {eligible}/{samples} scroll frames |")

    lines += [
        "",
        "The test distinguishes two things: the cost of a normal *single* Vello "
        "region, and eligibility to coalesce neighbours on a real editor frame. "
        "Synthetic 2/4/8/16-strip speedups must not be marketed as a natural "
        "scroll improvement when this report finds zero eligible regions.",
        "Reset includes worker-wake/barrier plus local setup; the Vello barrier "
        "must be instrumented *inside* the dispatcher to attribute its cost "
        "separately. Do not promote a worker threshold or new batch API until "
        "pixel parity, repeated A/B and cross-platform/memory gates pass.",
        "",
    ]
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("logs", nargs=3, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    report = summarize([load(p) for p in args.logs])
    args.output.write_text(report, encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
