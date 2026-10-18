#!/usr/bin/env python3
"""Summarize diagnostic Akula outline-atlas sample modes on one CI runner.

This is test-only: within-mode release p95 and cross-process screenshot
comparisons are evidence, not production benchmarks or visual acceptance.
"""
from __future__ import annotations

import statistics
import sys
from pathlib import Path

from PIL import Image, ImageChops, ImageStat

CASES = ("text_1x", "text_175", "cover_1x", "cover_175")
WORKERS = ("serial", "parallel")
STEPS = (0, 20)
SAMPLE_MODES = ("low", "high")


def metric_from_line(line: str) -> dict[str, str]:
    return dict(part.split("=", 1) for part in line.split() if "=" in part)


def parse_logs(directory: Path, mode: str) -> dict[tuple[str, str], list[dict[str, str]]]:
    logs = sorted(directory.glob(f"perf-{mode}-*.log"))
    if len(logs) != 8:
        raise ValueError(f"{mode}: expected 8 independent trial logs, got {len(logs)}")
    records: dict[tuple[str, str], list[dict[str, str]]] = {
        (case, worker): [] for case in CASES for worker in WORKERS
    }
    for log in logs:
        lines = log.read_text(errors="replace").splitlines()
        samples = [metric_from_line(s.split("VELLO_RELEASE ", 1)[1])
                   for s in lines if "VELLO_RELEASE case=" in s]
        parity = [metric_from_line(s.split("VELLO_RELEASE_PARITY ", 1)[1])
                  for s in lines if "VELLO_RELEASE_PARITY case=" in s]
        if len(samples) != 8 or len(parity) != 4:
            raise ValueError(f"{log}: expected 8 samples/4 parity rows; got {len(samples)}/{len(parity)}")
        for row in parity:
            if row["serial_parallel_diff"] != "0" or int(row["partial_full_max_channel"]) > 1:
                raise ValueError(f"{log}: parity gate failed: {row}")
        for row in samples:
            key = row["case"], row["mode"]
            if key not in records:
                raise ValueError(f"{log}: unexpected case/mode {key}")
            records[key].append(row)
    for key, rows in records.items():
        if len(rows) != 8:
            raise ValueError(f"{mode}/{key}: expected eight observations, got {len(rows)}")
    return records


def pixel_diff(a: Path, b: Path) -> dict[str, float | int]:
    with Image.open(a) as a_png, Image.open(b) as b_png:
        first, second = a_png.convert("RGBA"), b_png.convert("RGBA")
        if first.size != second.size:
            raise ValueError(f"image sizes differ: {a} {first.size}, {b} {second.size}")
        result = ImageChops.difference(first, second)
        r, g, blue, alpha = result.split()
        max_chan = ImageChops.lighter(
            ImageChops.lighter(r, g), ImageChops.lighter(blue, alpha)
        )
        histogram = max_chan.histogram()
        changed = sum(histogram[1:])
        peak = max((i for i, count in enumerate(histogram) if count), default=0)
        return {
            "changed": changed,
            "pct_changed": 100.0 * changed / (first.width * first.height),
            "peak": peak,
            "mean": sum(ImageStat.Stat(result).mean) / 4,
        }


def main() -> None:
    root = Path(sys.argv[1])
    low = parse_logs(root, "low")
    high = parse_logs(root, "high")
    lines = [
        "# Akula cached outline sampler A/B",
        "",
        "Eight release-profile samples per mode, alternating LOW–HIGH–HIGH–LOW "
        "in four groups on the same CI runner. Each sample uses 31 timed frames. "
        "Do not add per-case percentiles to calculate a total.",
        "",
        "| UI scene / threads | Low frame p95 (µs) | High frame p95 (µs) | Frame delta | Low raster p95 (µs) | High raster p95 (µs) | Raster delta |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for worker in WORKERS:
            key = case, worker
            def median(mode_data: dict[tuple[str, str], list[dict[str, str]]], field: str) -> float:
                return statistics.median(float(row[field]) for row in mode_data[key])
            fl, fh = median(low, "frame_p95_us"), median(high, "frame_p95_us")
            rl, rh = median(low, "raster_p95_us"), median(high, "raster_p95_us")
            lines.append(
                f"| {case} / {worker} | {fl:.2f} | {fh:.2f} | {(fh/fl-1)*100:+.1f}% | "
                f"{rl:.2f} | {rh:.2f} | {(rh/rl-1)*100:+.1f}% |"
            )
    lines.extend([
        "",
        "## Captured scene fidelity (cached versus direct, screen RGBA)",
        "",
        "All comparisons are across independent UI processes. Direct rendering is "
        "captured twice as a determinism control; if that control differs, "
        "cross-mode pixel comparisons are indicative only.",
        "",
        "| Scene / frame | Direct vs direct changed | Low vs direct changed | High vs direct changed | Low peak | High peak |",
        "|---|---:|---:|---:|---:|---:|",
    ])
    unstable = []
    for case in CASES:
        for step in STEPS:
            fname = f"{case}-step{step}.png"
            ref = root / "images" / "direct1" / fname
            ref2 = root / "images" / "direct2" / fname
            control = pixel_diff(ref, ref2)
            if control["changed"]:
                unstable.append(f"{case}/step{step}: {control['changed']} px, peak={control['peak']}")
            ldiff = pixel_diff(ref, root / "images" / "low" / fname)
            hdiff = pixel_diff(ref, root / "images" / "high" / fname)
            lines.append(
                f"| {case}/step{step} | {control['changed']} | {ldiff['changed']} "
                f"({ldiff['pct_changed']:.3f}%) | {hdiff['changed']} "
                f"({hdiff['pct_changed']:.3f}%) | {ldiff['peak']} | {hdiff['peak']} |"
            )
    lines.extend(["", "## Interpretation guardrails", ""])
    lines.append("- `High` uses a 4×4 bicubic sampler instead of nearest-neighbor, not a free numerical-precision fix.")
    lines.append("- 0/20 guard96 mismatches is necessary but not sufficient to ship; review images, p95, Windows/macOS UI parity.")
    if unstable:
        lines.append("- **Direct-control images were not deterministic:** " + "; ".join(unstable))
        lines.append("- Do not infer a visual-quality winner from cross-process pixel differences.")
    else:
        lines.append("- Direct-control images were byte-identical for all sampled frames.")
    output = root / "sampler-abba-summary.md"
    output.write_text("\n".join(lines) + "\n")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
