#!/usr/bin/env python3
"""Summarize release-mode 0/2/4-worker crossovers; fail on missing measurements."""

from __future__ import annotations

import argparse
import re
from pathlib import Path

CASES = ("text_1x", "text_175", "cover_1x", "cover_175")
WORKLOADS = ("normal", "guard96", "viewport", "full")
WORKERS = (0, 2, 4)

LINE = re.compile(
    r"VELLO_WORKER_CROSSOVER case=(?P<case>\w+) "
    r"workload=(?P<workload>\w+) workers=(?P<workers>\d+) "
    r"frames=(?P<frames>\d+) "
    r"total_p50_us=(?P<total>\d+(?:\.\d+)?) "
    r"total_p95_us=(?P<p95>\d+(?:\.\d+)?) "
    r"encode_p50_us=(?P<encode>\d+(?:\.\d+)?) "
    r"encode_p95_us=\d+(?:\.\d+)? "
    r"raster_p50_us=(?P<raster>\d+(?:\.\d+)?) "
    r"raster_p95_us=\d+(?:\.\d+)? "
    r"raster_area_p50_pct=(?P<area>\d+(?:\.\d+)?) "
    r"raster_area_p95_pct=\d+(?:\.\d+)?"
)


def read_results(files: list[Path]) -> dict[tuple[str, str, int], dict[str, str]]:
    results: dict[tuple[str, str, int], dict[str, str]] = {}
    for file in files:
        for match in LINE.finditer(file.read_text(encoding="utf-8")):
            fields = match.groupdict()
            key = (fields["workload"], fields["case"], int(fields["workers"]))
            if key in results:
                raise ValueError(f"duplicate worker sample {key} in {file}")
            if int(fields["frames"]) < 20:
                raise ValueError(f"too few scroll frames for {key}")
            results[key] = fields
    expected = {(w, c, n) for w in WORKLOADS for c in CASES for n in WORKERS}
    missing, unexpected = expected - results.keys(), results.keys() - expected
    if missing or unexpected:
        raise ValueError(
            f"missing ({len(missing)}): {sorted(missing)}; unexpected: {sorted(unexpected)}"
        )
    return results


def report(results: dict[tuple[str, str, int], dict[str, str]]) -> str:
    rows = [
        "# Vello worker crossover — release-mode Linux",
        "",
        "Same scene and physical frame in each 0/2/4-worker variant. "
        "ABC/BCA/CAB rotation reduces order effects. Timings use frames with detected scroll candidates (even when test modes suppress row copying).",
        "Area is *scheduled* raster work, including overlapping rectangles; "
        "it is not deduplicated damaged area. Worker count here is an "
        "experiment, not a production setting.",
        "",
        "| Workload | Case | Raster area p50 | 0 workers p50 | 2 workers p50 | 4 workers p50 | Best | Gain vs 2 |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for workload in WORKLOADS:
        for case in CASES:
            entries = [results[(workload, case, n)] for n in WORKERS]
            total = [float(x["total"]) for x in entries]
            best = min(range(len(total)), key=total.__getitem__)
            gain = (total[1] - total[best]) / total[1] * 100
            area = float(entries[0]["area"])
            rows.append(
                f"| {workload} | {case} | {area:.2f}% "
                f"| {total[0]:.1f} μs | {total[1]:.1f} μs "
                f"| {total[2]:.1f} μs | {WORKERS[best]} "
                f"| {gain:.1f}% |"
            )
    rows += [
        "",
        "Interpretation: compare within each workload and scene, not between "
        "different runners. This reports observed crossover candidates, "
        "not a universal switching threshold. Require repeat trials and "
        "independent whole-frame performance/fidelity gates before changing defaults.",
        "",
    ]
    return "\n".join(rows)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("logs", nargs=4, type=Path)
    parser.add_argument("--output", type=Path, default=Path("vello-worker-crossover-summary.md"))
    args = parser.parse_args()
    result = report(read_results(args.logs))
    args.output.write_text(result, encoding="utf-8")
    print(result)


if __name__ == "__main__":
    main()
