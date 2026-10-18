#!/usr/bin/env python3
"""Summarize cold-cache and steady-scrolling timings for the experimental outline path.

Diagnostic-only: never reclassify a >1/255 partial/full breach as a pass.
Compare medians of eight independent process-level samples, not pooled frames.
"""
from __future__ import annotations

import collections
import pathlib
import statistics
import sys

CASES = ("text_1x", "text_175", "cover_1x", "cover_175")
THREADS = ("serial", "parallel")
MODES = ("baseline", "canonical")


def parse_fields(line: str, marker: str) -> dict[str, str]:
    return dict(
        token.split("=", 1)
        for token in line.split(marker, 1)[1].split()
        if "=" in token
    )


def load_mode(root: pathlib.Path, mode: str) -> dict[str, object]:
    logs = sorted(root.glob(f"{mode}-*.log"))
    if len(logs) != 8:
        raise ValueError(f"{mode}: expected 8 independent process logs, got {len(logs)}")
    data: dict[str, object] = {
        "warm": collections.defaultdict(list),
        "cold": collections.defaultdict(list),
        "breaches": collections.Counter(),
        "max_delta": collections.Counter(),
        "applied": 0,
        "fallback": 0,
        "processes": len(logs),
    }
    for filename in logs:
        log = filename.read_text(errors="replace")
        perf = [
            parse_fields(line, "VELLO_RELEASE ")
            for line in log.splitlines() if "VELLO_RELEASE case=" in line
        ]
        cold = [
            parse_fields(line, "VELLO_CANONICAL_COLD ")
            for line in log.splitlines() if "VELLO_CANONICAL_COLD case=" in line
        ]
        fidelity = [
            parse_fields(line, "VELLO_RELEASE_PARITY ")
            for line in log.splitlines() if "VELLO_RELEASE_PARITY case=" in line
        ]
        if len(perf) != 8 or len(cold) != 4 or len(fidelity) != 4:
            raise ValueError(f"{filename}: incomplete samples perf={len(perf)}, cold={len(cold)}, fidelity={len(fidelity)}")
        if any(f["serial_parallel_diff"] != "0" or f["partial_full_tolerance"] != "1" for f in fidelity):
            raise ValueError(f"{filename}: bad serial/parallel parity or altered tolerance")
        for row in perf:
            key = (row["case"], row["mode"])
            if key[0] not in CASES or key[1] not in THREADS:
                raise ValueError(f"{filename}: unexpected perf key {key}")
            data["warm"][key].append(row)
        for row in cold:
            if row["case"] not in CASES:
                raise ValueError(f"{filename}: unexpected cold case: {row}")
            data["cold"][row["case"]].append(row)
        for row in fidelity:
            case = row["case"]
            delta = int(row["partial_full_max_channel"])
            if delta > 1:
                data["breaches"][case] += 1
            data["max_delta"][case] = max(data["max_delta"][case], delta)
        if "VELLO_CANONICAL_OUTLINE applied " in log:
            data["applied"] += 1
        if "VELLO_CANONICAL_OUTLINE fallback=" in log:
            data["fallback"] += 1

    for case in CASES:
        if len(data["cold"][case]) != 8:
            raise ValueError(f"{mode}/{case}: insufficient cold samples")
        for threads in THREADS:
            if len(data["warm"][(case, threads)]) != 8:
                raise ValueError(f"{mode}/{case}/{threads}: insufficient warm samples")
    return data


def median(items: list[dict[str, str]], key: str) -> float:
    return statistics.median(float(row[key]) for row in items)


def main(root: pathlib.Path) -> None:
    baseline, candidate = (load_mode(root, mode) for mode in MODES)
    lines = [
        "# Canonical local outline cache-miss raster: paired release A/B",
        "",
        "Four LOW–CANONICAL–CANONICAL–LOW cycles per runner, eight independent",
        "processes per arm and 31 timed steady-scroll frames per scene. Each",
        "process starts with a fresh glyph cache. The first full frame is",
        "reported separately as a cold-cache sample.",
        "",
        "## First-frame cold-cache cost",
        "",
        "| Case / worker | Baseline (µs) | Canonical (µs) | Delta |",
        "|---|---:|---:|---:|",
    ]
    for case in CASES:
        for threads in THREADS:
            metric = f"{threads}_frame_us"
            a = median(baseline["cold"][case], metric)
            b = median(candidate["cold"][case], metric)
            lines.append(f"| {case} / {threads} | {a:.2f} | {b:.2f} | {(b/a-1)*100:+.1f}% |")

    lines += [
        "",
        "## Steady scrolling (median of per-process p95)",
        "",
        "| Case / worker | Baseline frame p95 (µs) | Canonical frame p95 (µs) | Frame delta | Baseline raster p95 (µs) | Canonical raster p95 (µs) | Raster delta |",
        "|---|---:|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for threads in THREADS:
            key = (case, threads)
            fa = median(baseline["warm"][key], "frame_p95_us")
            fb = median(candidate["warm"][key], "frame_p95_us")
            ra = median(baseline["warm"][key], "raster_p95_us")
            rb = median(candidate["warm"][key], "raster_p95_us")
            lines.append(
                f"| {case} / {threads} | {fa:.2f} | {fb:.2f} | {(fb/fa-1)*100:+.1f}% | "
                f"{ra:.2f} | {rb:.2f} | {(rb/ra-1)*100:+.1f}% |"
            )
    lines += [
        "",
        "## Fidelity observed during timings — NOT an acceptance gate",
        "",
        "| Case | Baseline >1/255 processes | Canonical >1/255 processes | Baseline peak | Canonical peak |",
        "|---|---:|---:|---:|---:|",
    ]
    for case in CASES:
        lines.append(
            f"| {case} | {baseline['breaches'][case]}/8 | {candidate['breaches'][case]}/8 | "
            f"{baseline['max_delta'][case]}/255 | {candidate['max_delta'][case]}/255 |"
        )
    lines += [
        "",
        f"**Treatment coverage:** canonical raster applied in {candidate['applied']}/8 processes; "
        f"fallback detected in {candidate['fallback']}/8.",
        "",
        "## Conditions",
        "",
        "- These are first-frame and warm-scroll **UI harness** timings, not the entire app startup or a large-file import.",
        "- A >1/255 breach is still a **failure**; timing-only mode reports it instead of aborting other cases. The strict guard96 workflow remains independent and unchanged.",
        "- Check warm reuse and cold overhead on real books, cross-platform font rendering and atlas memory before shipping. Do not treat fewer errors on a few frames as proof of universal parity.",
    ]
    summary = "\n".join(lines) + "\n"
    (root / "summary.md").write_text(summary)
    print(summary)


if __name__ == "__main__":
    main(pathlib.Path(sys.argv[1]))
