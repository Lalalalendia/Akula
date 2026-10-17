#!/usr/bin/env python3
"""Compare Vello worker routing across 2/4/8/16 full-frame damage strips."""

import argparse
import re
import statistics
from pathlib import Path

ROW = re.compile(
    r"VELLO_FRAGMENTED_ROUTE case=(\w+) full_every=(\d+) mode=(\w+) "
    r"workload=(\w+) frames=(\d+) strips=(\d+) per_rect_switches=(\d+) "
    r"frame_switches=(\d+) p50_us=([\d.]+) p95_us=([\d.]+) total_us=([\d.]+)"
)
CASES = ("text_1x", "text_175", "cover_175")
STRIPS = (2, 4, 8, 16)
MODES = ("serial", "parallel2", "per_rect", "frame_budget")
EVERY = (8, 3)
CLASSES = ("scroll", "full")
EXPECTED = {
    (strips, case, interval, mode, segment)
    for strips in STRIPS for case in CASES for interval in EVERY
    for mode in MODES for segment in CLASSES
}
FRAME_SIZE = {
    "text_1x": (1000, 600),
    "text_175": (1750, 1051),
    "cover_175": (1750, 1051),
}
THRESHOLD_PX = 350_000


def parse_trial(file: Path) -> dict[tuple[int, str, int, str, str], tuple[int, float]]:
    records = {}
    content = file.read_text(encoding="utf-8")
    for m in ROW.finditer(content):
        case, every, mode, area, frames, strips, per_rect, frame, p50, p95, total = m.groups()
        count = int(strips)
        key = count, case, int(every), mode, area
        if key in records:
            raise ValueError(f"{file}: duplicate measurement: {key}")
        if int(frames) < 4 or count not in STRIPS or int(frame) < 6:
            raise ValueError(f"{file}: missing frames or mode switches: {key}")
        width, height = FRAME_SIZE[case]
        largest = width * ((height + count - 1) // count)
        if largest > THRESHOLD_PX:
            valid = int(per_rect) >= 6
        else:
            valid = int(per_rect) <= 2
        if not valid:
            raise ValueError(f"{file}: wrong per-rectangle route for {key}: {per_rect}")
        records[key] = int(frames), float(total)
    if set(records) != EXPECTED:
        raise ValueError(
            f"{file}: missing ({len(EXPECTED - set(records))}): "
            f"{sorted(EXPECTED - set(records))}; extra: {sorted(set(records) - EXPECTED)}"
        )
    return records


def summarize(trials: list[dict]) -> str:
    lines = [
        "# Vello worker routing under fragmented full-frame damage",
        "",
        "Three separate release-mode Linux test processes. Real editor scenes, "
        "same framebuffer for fixed 0/2 and both routing strategies.",
        "Each full redraw is split into 2, 4, 8 or 16 **disjoint** bands.",
        "All framebuffers must remain byte-identical across worker modes and "
        "within 1 LSB of the independent unsplit full reference (Rust gate).",
        "The experimental threshold is 350k pixels. Segment totals sum actual "
        "sampled elapsed frame times; never add p50 medians.",
        "",
        "| Strips | Scene | Full every | Serial total | Two-worker total | Per-rect total | Frame-budget total | Frame vs rect median gain (range) |",
        "|---:|---|---:|---:|---:|---:|---:|---:|",
    ]
    scores = {n: {"wins": 0, "count": 0, "against_static": 0} for n in STRIPS}
    for count in STRIPS:
        for case in CASES:
            for every in EVERY:
                records = [
                    {
                        mode: sum(trial[(count, case, every, mode, area)][1] for area in CLASSES)
                        for mode in MODES
                    }
                    for trial in trials
                ]
                by_mode = {
                    mode: statistics.median(rec[mode] for rec in records)
                    for mode in MODES
                }
                gains = [
                    (x["per_rect"] - x["frame_budget"]) / x["per_rect"] * 100.0
                    for x in records
                ]
                for rec, gain in zip(records, gains):
                    scores[count]["wins"] += int(gain > 0)
                    scores[count]["count"] += 1
                    static_best = min(rec["serial"], rec["parallel2"])
                    scores[count]["against_static"] += int(rec["frame_budget"] < static_best)
                lines.append(
                    f"| {count} | {case} | {every} "
                    f"| {by_mode['serial']/1000:.2f} ms "
                    f"| {by_mode['parallel2']/1000:.2f} ms "
                    f"| {by_mode['per_rect']/1000:.2f} ms "
                    f"| {by_mode['frame_budget']/1000:.2f} ms "
                    f"| {statistics.median(gains):+.1f}% ({min(gains):+.1f}…{max(gains):+.1f}%) |"
                )
    lines += [
        "",
        "## Evidence by fragmentation",
        "",
        "| Strips | Frame-budget beats per-rect | Frame-budget beats best static |",
        "|---:|---:|---:|",
    ]
    for strips in STRIPS:
        n = scores[strips]
        lines.append(
            f"| {strips} | {n['wins']}/{n['count']} "
            f"| {n['against_static']}/{n['count']} |"
        )
    lines += [
        "",
        "This is a test-only diagnostic, not a deployment threshold: "
        "further controls must include cache memory/RSS, diverse rectangle "
        "geometry, Windows/macOS and a complete native UI benchmark.",
        "",
    ]
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("logs", nargs=3, type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = summarize([parse_trial(p) for p in args.logs])
    args.output.write_text(result, encoding="utf-8")
    print(result)


if __name__ == "__main__":
    main()
