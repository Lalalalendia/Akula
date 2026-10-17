#!/usr/bin/env python3
"""Compare existing native note latency matrices on one hosted runner.

ABBA uses the same commit and test process per run, toggling only the
test-only Parley layout gate. These are still noisy debug-profile samples.
"""
from __future__ import annotations

import re
import statistics
from pathlib import Path

CASES = ("baseline-1", "candidate-1", "candidate-2", "baseline-2")
SIZES = ("64", "512", "4096")


def samples(case: str) -> dict[str, dict[str, str]]:
    text = Path(f"parley-tab-{case}.log").read_text(encoding="utf-8")
    records = {}
    for line in text.splitlines():
        if "NOTE_LATENCY kib=" not in line:
            continue
        pairs = dict(re.findall(r"([a-z_]+)=([^\s]+)", line.split("NOTE_LATENCY ", 1)[1]))
        size = pairs["kib"]
        if size in records:
            raise ValueError(f"Duplicated size {size} in {case}")
        if int(pairs["cache_hits"]) != int(pairs["cycles"]):
            raise ValueError(f"Missing Rope cache hits in {case} at {size} KiB")
        if int(pairs["stale_overwrites"]) != 0:
            raise ValueError(f"Stale overwrite in {case} at {size} KiB")
        if case.startswith("baseline") and (
            int(pairs["parley_offered"]) or int(pairs["parley_reused"])
        ):
            raise ValueError(f"Baseline silently reused Parley at {size} KiB")
        records[size] = pairs
    if set(records) != set(SIZES):
        raise ValueError(f"Incomplete note matrix in {case}: {sorted(records)}")
    return records


def main() -> None:
    data = {case: samples(case) for case in CASES}
    print("### Clean-tab Parley cache, whole-UI switch ABBA")
    print()
    print("One GitHub-hosted Ubuntu runner, same final commit and test profile.")
    print("Two baseline and two cached runs, each with 16 switches per note size.")
    print()
    print("| Size | Baseline switch p95, ms | Cached switch p95, ms | Candidate / baseline | Cached offered/reused |")
    print("|---|---:|---:|---:|---:|")
    for kib in SIZES:
        before = statistics.median(
            float(data[c][kib]["switch_p95_ms"]) for c in CASES if c.startswith("baseline")
        )
        after = statistics.median(
            float(data[c][kib]["switch_p95_ms"]) for c in CASES if c.startswith("candidate")
        )
        offered = sum(
            int(data[c][kib]["parley_offered"]) for c in CASES if c.startswith("candidate")
        )
        reused = sum(
            int(data[c][kib]["parley_reused"]) for c in CASES if c.startswith("candidate")
        )
        print(
            f"| {kib} KiB | {before:.2f} | {after:.2f} | "
            f"{100 * after / before:.1f}% | {offered}/{reused} |"
        )
        if kib == "4096" and reused <= 0:
            raise AssertionError("No real Parley reuse occurred for 4 MiB switching")
    print()
    print("Do not sum stage p95s or interpret this diagnostic as a release-build")
    print("guarantee. Paired samples expose both gains and regressions; no")
    print("performance threshold is used to bypass functionality or fidelity tests.")


if __name__ == "__main__":
    main()
