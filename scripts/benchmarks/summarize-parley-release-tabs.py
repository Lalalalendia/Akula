#!/usr/bin/env python3
"""Summarize paired release-profile UI Parley ABBA runs without hiding noise."""
from __future__ import annotations

import re
import statistics
import sys
from pathlib import Path

SIZES = (64, 512, 4096)
PASSES = 3


def main() -> None:
    source = Path(sys.argv[1]).read_text(encoding="utf-8")
    data: dict[int, list[dict[str, str]]] = {size: [] for size in SIZES}
    for line in source.splitlines():
        if not line.startswith("PARLEY_RELEASE_NOTE "):
            continue
        fields = dict(re.findall(r"([a-z0-9_]+)=([^\s]+)", line))
        size = int(fields["kib"])
        if size not in data:
            raise ValueError(f"Unexpected test size {size}")
        if fields["profile"] != "release" or int(fields["samples"]) != 24:
            raise ValueError(f"Invalid release matrix fields at {size} KiB")
        if int(fields["parley_reused"]) <= 0 or int(fields["rope_hits"]) != 52:
            raise ValueError(f"No real Parley reuse or missing Rope verification at {size} KiB")
        data[size].append(fields)
    for size, records in data.items():
        if len(records) != PASSES:
            raise ValueError(f"Expected {PASSES} release passes at {size} KiB; got {len(records)}")

    print("### Parley real native tab switches — repeated release-profile ABBA")
    print()
    print("3 same-runner passes; each case has 24 measured baseline and 24 cached switches.")
    print("Timings include the UI switch + first Masonry redraw, not GPU / OS composition.")
    print("Numbers are CI-runner observations, not a production speed guarantee.")
    print()
    print("| Note | Baseline p95 median | Cached p95 median | Median paired p95 ratio | Paired ratios |")
    print("|---|---:|---:|---:|---|")
    paired: dict[int, list[float]] = {}
    for size in SIZES:
        cases = data[size]
        baseline = statistics.median(float(r["baseline_p95_ms"]) for r in cases)
        cached = statistics.median(float(r["candidate_p95_ms"]) for r in cases)
        ratios = [
            float(r["candidate_p95_ms"]) / float(r["baseline_p95_ms"])
            for r in cases
        ]
        paired[size] = ratios
        variants = ", ".join(f"{v:.3f}x" for v in ratios)
        print(
            f"| {size} KiB | {baseline:.2f} ms | {cached:.2f} ms | "
            f"{statistics.median(ratios):.3f}x | {variants} |"
        )

    # This recommendation is deliberately conservative: a single good 4 MiB
    # sample must not outweigh regressions and must not enable product code.
    useful_large = statistics.median(paired[4096]) <= 0.90 and all(
        ratio < 1.0 for ratio in paired[4096]
    )
    small_safe = all(statistics.median(paired[size]) <= 1.05 for size in (64, 512))
    decision = "ELIGIBLE_FOR_FURTHER_PRODUCT_VALIDATION" if useful_large and small_safe else "HOLD_PRODUCTION_CACHE"
    print()
    print(f"PARLEY_RELEASE_DECISION={decision}")
    print("A favorable diagnostic still requires #117 to pass final-head")
    print("production-code pixel, editing, scroll, and lifecycle tests.")
    print("This script never merges, deploys, or enables Parley caching.")


if __name__ == "__main__":
    main()
