#!/usr/bin/env python3
"""AKULA M2: real Win32 GUI idle-memory witness (diagnostic only).

Reuses the existing M1 PSAPI sampler; 60 s settle followed by 600 s idle.
All profiles and the empty vault are synthetic and created under --out.
It does not validate installed NSIS, UIA first-interactive or 5-run medians.
"""
from __future__ import annotations

import argparse
import csv
import json
import os
from pathlib import Path
from statistics import median
import sys

from native_windows_memory_pilot import hash_file, run_one

SETTLE_S = 60
IDLE_S = 600
TOTAL_S = SETTLE_S + IDLE_S
INTERVAL_MS = 1000
MIB = 1048576


def metric(row: dict, key: str) -> int:
    return int(row[key])


def summary(rows: list[dict], witness: dict, manifest: dict) -> tuple[dict, str]:
    if not witness["graceful_exit"] or witness["forced_kill"] or not witness["window"]:
        raise RuntimeError("Missing native HWND or clean process shutdown")
    if not rows:
        raise RuntimeError("No process observations")
    times = [float(r["elapsed_s"]) for r in rows]
    if any(a >= b for a, b in zip(times, times[1:])):
        raise RuntimeError("Non-monotonic sample timestamps")
    if times[0] > 3 or times[-1] < TOTAL_S - 2:
        raise RuntimeError(f"Incomplete idle observation interval: {times[0]}..{times[-1]}")
    if len({r["pid"] for r in rows}) != 1 or len({r["created_ticks"] for r in rows}) != 1:
        raise RuntimeError("PID or process identity changed during measurement")

    idle = [r for r in rows if SETTLE_S <= float(r["elapsed_s"]) < TOTAL_S]
    early = [r for r in idle if float(r["elapsed_s"]) < SETTLE_S + 30]
    late = [r for r in idle if float(r["elapsed_s"]) >= TOTAL_S - 30]
    if len(idle) < 585 or len(early) < 24 or len(late) < 24:
        raise RuntimeError("Insufficient complete idle/early/late samples")

    private = lambda r: metric(r, "private_bytes") / MIB
    rss = lambda r: metric(r, "working_set_bytes") / MIB
    initial = median(private(r) for r in early)
    last = median(private(r) for r in late)
    elapsed = [float(r["elapsed_s"]) - SETTLE_S for r in idle]
    values = [private(r) for r in idle]
    xmean = sum(elapsed) / len(elapsed)
    ymean = sum(values) / len(values)
    denominator = sum((t - xmean) ** 2 for t in elapsed)
    slope = (sum((t - xmean) * (v - ymean) for t, v in zip(elapsed, values)) /
             denominator * 3600) if denominator else 0.0
    cpu_start = metric(early[0], "cpu_ms")
    cpu_end = metric(late[-1], "cpu_ms")
    if cpu_end < cpu_start:
        raise RuntimeError("CPU counter decreased unexpectedly")

    report = {
        "idle_duration_s": IDLE_S, "settle_duration_s": SETTLE_S,
        "idle_samples": len(idle), "gui_visible_proxy_s": witness["visible_hwnd_proxy_s"],
        "private_mib_early_p50": round(initial, 3),
        "private_mib_late_p50": round(last, 3),
        "private_mib_tail_delta": round(last - initial, 3),
        "private_mib_idle_sample_min": round(min(values), 3),
        "private_mib_idle_sample_max": round(max(values), 3),
        "private_mib_idle_linear_trend_per_hour": round(slope, 3),
        "working_set_mib_early_p50": round(median(rss(r) for r in early), 3),
        "working_set_mib_late_p50": round(median(rss(r) for r in late), 3),
        "peak_working_set_mib": round(max(metric(r, "peak_working_set_bytes") for r in rows) / MIB, 3),
        "cpu_ms_during_idle": cpu_end - cpu_start,
        "handles_early_p50": round(median(metric(r, "handles") for r in early), 1),
        "handles_late_p50": round(median(metric(r, "handles") for r in late), 1),
        "process_clean_exit": True,
    }
    lines = [
        "# Akula M2 10-minute idle Win32 diagnostic", "",
        "**One synthetic empty-vault, new-profile process. No physical NSIS-installed RAM baseline.**",
        "The first uncloaked HWND does not prove first usable UI interaction.",
        "A valid instrument run is not a declaration of zero leaks or a production budget.", "",
        f"Source SHA: `{manifest['source_sha']}`; EXE SHA-256: `{manifest['binary_sha256']}`.",
        f"Observations: {SETTLE_S} s settle then {IDLE_S} s idle; {len(idle)} actual idle samples (nominal 1 second).",
        "",
        "| Quantity | Observed |", "|---|---:|",
        f"| Median Private Bytes, first 30s of idle | {report['private_mib_early_p50']:.3f} MiB |",
        f"| Median Private Bytes, last 30s of idle | {report['private_mib_late_p50']:.3f} MiB |",
        f"| Last minus first Private Bytes median | {report['private_mib_tail_delta']:+.3f} MiB |",
        f"| Sampled Private Bytes min–max during idle | {report['private_mib_idle_sample_min']:.3f}–{report['private_mib_idle_sample_max']:.3f} MiB |",
        f"| Linear Private Bytes trend (descriptive, not inferential) | {report['private_mib_idle_linear_trend_per_hour']:+.3f} MiB/h |",
        f"| Median Working Set (first/last 30s of idle) | {report['working_set_mib_early_p50']:.3f}/{report['working_set_mib_late_p50']:.3f} MiB |",
        f"| Windows high-water process Working Set | {report['peak_working_set_mib']:.3f} MiB |",
        f"| CPU time accumulated during idle window | {report['cpu_ms_during_idle']} ms |",
        f"| Process handles first/last p50 | {report['handles_early_p50']:.1f}/{report['handles_late_p50']:.1f} |",
        "",
        "Private Bytes is process private commit, not exclusive RAM or GPU allocation.",
        "A straight-line fit across one ten-minute trace is descriptive only.",
        "A separate Windows installed NSIS + 5 runs is required for M2 production acceptance.",
        "Do not compare directly to development-only mmap, WARP and alternate-viewport tests.",
        "",
    ]
    return report, "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("A real Windows GUI process is required")
    if not args.binary.is_file():
        raise RuntimeError("Release executable not found")
    if args.out.exists() and any(args.out.iterdir()):
        raise RuntimeError("Refusing nonempty output directory; user data is never a fixture")
    args.out.mkdir(parents=True, exist_ok=True)
    manifest = {
        "source_sha": os.getenv("AKULA_PROBE_SOURCE_HEAD", "unknown"),
        "binary_sha256": hash_file(args.binary),
        "mode": "unpackaged Windows Release --features installed; synthetic empty vault",
        "settle_s": SETTLE_S, "idle_s": IDLE_S, "sampling_interval_ms": INTERVAL_MS,
    }
    (args.out / "environment.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    print("AKULA_M2_START scenario=empty_vault_new_profile idle=600s", flush=True)
    run_one(args.binary.resolve(), args.out.resolve(), "empty_vault_cold", 1, TOTAL_S, INTERVAL_MS)
    where = args.out / "trial-1" / "empty_vault_cold"
    with (where / "samples.csv").open(encoding="utf-8", newline="") as f:
        rows = list(csv.DictReader(f))
    witness = json.loads((where / "witness.json").read_text(encoding="utf-8"))
    outcome, text = summary(rows, witness, manifest)
    (args.out / "results.json").write_text(json.dumps(outcome, indent=2), encoding="utf-8")
    (args.out / "summary.md").write_text(text, encoding="utf-8")
    print("AKULA_M2_PASS genuine_win32=1 idle_samples=" + str(outcome["idle_samples"]), flush=True)
    print(text, flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"AKULA_M2_FAIL {exc!r}", file=sys.stderr, flush=True)
        raise
