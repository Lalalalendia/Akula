#!/usr/bin/env python3
"""IMG-PERF-09: exact-source two-frame vs 512-frame mip-cache lifetime.

Real Win32 GUI, same Medium sampler, scalar reducer, source and cache budget.
Only the developer cache expiry flag differs. Installed Akula untouched.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path
from statistics import median

from native_mipmap_window_ab import SOURCE, run_one
from native_mipmap_windows_ab import (
    windows_scenes, replay_work, percentile, stage_samples,
)

MODES = ("mip_medium", "mip_medium_linger")
TRIALS = (1, 2, 3)


def statistics(row: dict) -> dict:
    work = replay_work(row)
    whole, replay = stage_samples(row)
    if len(replay) != len(work) or len(whole) < len(replay):
        raise ValueError("Misaligned frame-stage samples")
    generated = sum(int(x["mipGenerated"]) for x in whole)
    after_start = sum(int(x["mipGenerated"]) for x in replay)
    replay_events = sum(int(x["mipGenerated"] > 0) for x in replay)
    whole_ms = sum(float(x["mipBuildMs"]) for x in whole)
    replay_ms = sum(float(x["mipBuildMs"]) for x in replay)
    if generated < 2 or generated % 2 or after_start % 2:
        raise ValueError("Invalid 2-level mip construction evidence")
    if not 0 <= replay_ms <= whole_ms + 1e-7:
        raise ValueError("Bad replay vs whole-process mip timing")
    return {
        "cpu_p95_ms": percentile(work, .95),
        "cpu_p99_ms": percentile(work, .99),
        "cpu_mean_ms": sum(work) / len(work),
        "mip_generated_levels": generated,
        "replay_mip_levels": after_start,
        "replay_build_events": replay_events,
        "replay_mip_ms": replay_ms,
        "whole_mip_ms": whole_ms,
        "rss_mib": row["peak_rss_kib"] / 1024,
        "replay_frames": len(work),
    }


def make_report(rows: list[dict], binary_sha: str, scenes: tuple[tuple[str, str], ...]) -> str:
    expected = len(TRIALS) * len(MODES) * len(scenes)
    if len(rows) != expected:
        raise ValueError(f"Incomplete 12-run Win32 lifespan matrix ({len(rows)}/{expected})")
    lines = [
        "# IMG-PERF-09: 2 vs 512 frame mip-cache retention",
        "", f"Developer release binary SHA256: {binary_sha}.",
        "Same Medium Vello filter, 2-level scalar mip reduction, source pixels,",
        "original 32MiB budget, 2 threads, 4s native replay, both window sizes.",
        "Only AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER differs between paired modes.",
        "Mipmap mode must be used and both levels generated at least once.",
        "These are CPU work times, not screen refresh rate or installed FPS.",
        "",
        "| Viewport | Mode | CPU p95 ms | CPU p99 ms | Replay mip builds | Replay mip CPU ms | Whole-process mip CPU ms | Peak RSS MiB |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for scene, viewport in scenes:
        subset = [r for r in rows if r["scene"] == scene]
        for field in ("note_sha256", "image_sha256"):
            if len({r[field] for r in subset}) != 1:
                raise ValueError(f"Input mismatch {scene}/{field}")
        sums = {}
        for mode in MODES:
            group = [r for r in subset if r["mode"] == mode]
            if sorted(int(r["trial"]) for r in group) != list(TRIALS):
                raise ValueError(f"Unpaired trials for {scene}/{mode}")
            vals = []
            for row in group:
                if row["applied"] is not True:
                    raise ValueError(f"Mipmap path not exercised: {scene}/{mode}")
                witness = row.get("window_witness")
                if not isinstance(witness, dict) or witness.get("title_akula") is not True \
                        or witness.get("uncloaked") is not True:
                    raise ValueError(f"Missing native Win32 HWND: {scene}/{mode}")
                vals.append(statistics(row))
            summary = {key: median(x[key] for x in vals) for key in vals[0]}
            sums[mode] = summary
            lines.append(
                f"| {viewport} | {mode} | {summary['cpu_p95_ms']:.4f} | "
                f"{summary['cpu_p99_ms']:.4f} | {summary['replay_build_events']:.0f} | "
                f"{summary['replay_mip_ms']:.3f} | {summary['whole_mip_ms']:.3f} | "
                f"{summary['rss_mib']:.1f} |"
            )
            # For this repeated-image pinned benchmark, distinct positive
            # replay-generation counts indicate lifetime churn.
            for run in vals:
                if mode == "mip_medium" and run["replay_build_events"] == 0:
                    raise ValueError(f"2-frame path never rebuilt during replay: {scene}")
                if mode == "mip_medium_linger" and run["replay_build_events"] != 0:
                    raise ValueError(f"512-frame path still rebuilt during replay: {scene}")
        lines.append(
            f"{viewport}: replay mip CPU saved by linger = "
            f"{sums['mip_medium']['replay_mip_ms'] - sums['mip_medium_linger']['replay_mip_ms']:.3f}ms "
            "(median of per-mode run metrics; descriptive)."
        )
        paired = []
        for trial in TRIALS:
            short = next(x for x in subset if x["trial"] == trial and x["mode"] == "mip_medium")
            long = next(x for x in subset if x["trial"] == trial and x["mode"] == "mip_medium_linger")
            base, opt = statistics(short), statistics(long)
            paired.append({
                "trial": trial,
                "cpu_p95_delta_ms": round(opt["cpu_p95_ms"] - base["cpu_p95_ms"], 4),
                "cpu_p99_delta_ms": round(opt["cpu_p99_ms"] - base["cpu_p99_ms"], 4),
                "replay_build_events": [base["replay_build_events"], opt["replay_build_events"]],
            })
        lines.append(f"{viewport} paired per-trial changes (linger - short): {json.dumps(paired)}")
        lines.append("")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--timeout", type=float, default=50)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Only genuine Win32 GUI evidence is valid")
    binary = args.binary.resolve(strict=True)
    if binary.suffix.lower() != ".exe":
        raise ValueError("Expected Windows native executable")
    if SOURCE.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Pinned image fixture missing")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    os.environ["AKULA_EXPERIMENTAL_FRAME_STAGES"] = "1"
    # No unrelated flags should vary sampling quality or select direct L2.
    for key in (
        "AKULA_EXPERIMENTAL_IMAGE_MIP_DIRECT_L2",
        "AKULA_EXPERIMENTAL_IMAGE_FILTER_ONLY",
        "AKULA_EXPERIMENTAL_IMAGE_FILTER_FORCE",
        "AKULA_EXPERIMENTAL_IMAGE_FORCE_FULL_REDRAW",
    ):
        os.environ.pop(key, None)
    scenes = windows_scenes()
    rows = []
    for trial in TRIALS:
        order = MODES if trial % 2 else tuple(reversed(MODES))
        for scene, size in scenes:
            for mode in order:
                rows.append(run_one(binary, out, trial, mode, scene, size, args.timeout))
    report = make_report(rows, hashlib.sha256(binary.read_bytes()).hexdigest(), scenes)
    (out / "summary.md").write_text(report, encoding="utf-8")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    print(report, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"AKULA_IMG_PERF_09_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
