#!/usr/bin/env python3
"""IMG-PERF-07: isolate Vello High/Medium from developer mipmap LOD.

This is developer-only CPU timing for a real Win32/Winit window; never infer
hardware scanout FPS or pixel fidelity from the timings.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from math import isfinite
from pathlib import Path
from statistics import median

from native_mipmap_window_ab import SOURCE, run_one
from native_mipmap_windows_ab import windows_scenes, replay_work, percentile, stage_samples

MODES = ("baseline", "filter_medium", "mip_high", "mip_medium")
TRIALS = (1, 2, 3)


def stat(row: dict) -> dict:
    work = replay_work(row)
    all_stage, replay_stages = stage_samples(row)
    if len(replay_stages) != len(work):
        raise ValueError("Stage samples must align with presented replay CPU work")
    if not all_stage or not replay_stages:
        raise ValueError("Missing cold or replay stages")
    return {
        "cpu_p95_ms": percentile(work, 0.95),
        "cpu_p99_ms": percentile(work, 0.99),
        "cpu_mean_ms": sum(work) / len(work),
        "vello_mean_ms": sum(float(s["velloMs"]) for s in replay_stages) / len(replay_stages),
        "encode_mean_ms": sum(float(s["encodeMs"]) for s in replay_stages) / len(replay_stages),
        "mip_build_ms": sum(float(s["mipBuildMs"]) for s in all_stage),
        "mip_build_replay_ms": sum(float(s["mipBuildMs"]) for s in replay_stages),
        "mip_build_replay_events": sum(int(s["mipGenerated"] > 0) for s in replay_stages),
        "mip_generated": sum(int(s["mipGenerated"]) for s in all_stage),
        "peak_rss_mib": row["peak_rss_kib"] / 1024,
    }


def report(rows: list[dict], binary_sha: str, scenes: tuple[tuple[str, str], ...]) -> str:
    if len(rows) != len(scenes) * len(MODES) * len(TRIALS):
        raise ValueError("Incomplete 4-mode native Win32 factorial A/B")
    lines = [
        "# IMG-PERF-07: Vello filter vs mipmap 2x2",
        "",
        f"Developer-only release binary SHA256: {binary_sha}.",
        "Single pinned PNG 1280x884: filter and mip factors vary independently.",
        "Modes: baseline High/no mip; Medium/no mip; mip+High; mip+Medium.",
        "Same renderer sample clocks and 4s scroll replay for every mode.",
        "WARNING: mipmap cells use original 2-frame cache lifetime (512-frame linger OFF).",
        "Whole-process mip costs INCLUDE replay rebuilds; they are NOT startup-only costs.",
        "No shipped pixel behavior or sampler defaults change.",
        "Pure Medium vs mip Medium is NOT a pixel-identical image-quality comparison.",
        "These are CPU-work durations, not visible monitor frame-pacing or scanout.",
        "",
        "| Scene | Size | Mode | CPU p95 (ms) | CPU p99 (ms) | Mean encode (ms) | Mean Vello (ms) | Whole-process mip (ms) | Replay mip (ms) | Replay rebuilds | RSS (MiB) |",
        "|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    allstats = {}
    for scene, size in scenes:
        scene_rows = [row for row in rows if row["scene"] == scene]
        for field in ("note_sha256", "image_sha256"):
            if len({row[field] for row in scene_rows}) != 1:
                raise ValueError(f"Fixture changed for {scene}/{field}")
        table = {}
        for mode in MODES:
            group = [row for row in scene_rows if row["mode"] == mode]
            if sorted(row["trial"] for row in group) != list(TRIALS):
                raise ValueError(f"Missing trial for {scene}/{mode}")
            for row in group:
                expected_mip = mode.startswith("mip_")
                expected_filter_only = mode == "filter_medium"
                if row["applied"] != expected_mip or row["filter_only_applied"] != expected_filter_only or row["forced_quality"] != ("medium" if mode in ("filter_medium", "mip_medium") else "high"):
                    raise ValueError(f"Missing or unexpected filter/mip witness: {scene}/{mode}")
                w = row.get("window_witness")
                if not isinstance(w, dict) or w.get("title_akula") is not True or w.get("uncloaked") is not True:
                    raise ValueError(f"Missing visible Win32 window witness: {scene}/{mode}")
                if int(w.get("width", 0)) < 400 or int(w.get("height", 0)) < 300:
                    raise ValueError("Invalid GUI window dimensions")
            samples = [stat(row) for row in group]
            med = {key: median(s[key] for s in samples) for key in samples[0]}
            table[mode] = med
            lines.append(
                f"| {scene} | {size} | {mode} | {med['cpu_p95_ms']:.3f} | "
                f"{med['cpu_p99_ms']:.3f} | {med['encode_mean_ms']:.3f} | "
                f"{med['vello_mean_ms']:.3f} | {med['mip_build_ms']:.3f} | "
                f"{med['mip_build_replay_ms']:.3f} | "
                f"{med['mip_build_replay_events']:.0f} | "
                f"{med['peak_rss_mib']:.1f} |"
            )
        allstats[scene] = table
        for field in ("cpu_p95_ms", "vello_mean_ms"):
            deltas = {}
            for trial in TRIALS:
                per = {r["mode"]: stat(r)[field] for r in scene_rows if r["trial"] == trial}
                if len(per) != len(MODES):
                    raise ValueError(f"Unpaired trial: {scene}/{trial}")
                # Filter effect without mip vs filter effect with mip.
                no_mip = per["filter_medium"] - per["baseline"]
                mip_filter = per["mip_medium"] - per["mip_high"]
                high_mip = per["mip_high"] - per["baseline"]
                medium_mip = per["mip_medium"] - per["filter_medium"]
                deltas[trial] = (no_mip, mip_filter, high_mip, medium_mip)
            lines.append(
                f"{scene} / {field}: paired deltas [filter/no-mip, filter/mip, "
                f"mip/High, mip/Medium] in milliseconds:"
            )
            for trial in TRIALS:
                values = deltas[trial]
                lines.append(f"- trial {trial}: {[round(v, 4) for v in values]}")
            interaction = [
                deltas[t][1] - deltas[t][0] for t in TRIALS
            ]
            lines.append(
                f"{scene} / {field}: filter×mip interaction (difference of differences) "
                f"{[round(d, 4) for d in interaction]}; descriptive only."
            )
        lines.append("")
    for scene, table in allstats.items():
        if table["baseline"]["mip_generated"] != 0 or table["filter_medium"]["mip_generated"] != 0:
            raise ValueError(f"Filter-only or baseline built mipmap for {scene}")
        if table["mip_high"]["mip_generated"] < 2 or table["mip_medium"]["mip_generated"] < 2:
            raise ValueError(f"Missing mip build for {scene}")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=50)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("This A/B requires a real Windows native HWND")
    binary = args.binary.resolve(strict=True)
    if binary.suffix.lower() != ".exe":
        raise ValueError("Expected native Windows executable")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    png = SOURCE.read_bytes()
    if png[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Pinned PNG fixture unavailable")
    scene_sizes = windows_scenes()
    exe_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    os.environ["AKULA_EXPERIMENTAL_FRAME_STAGES"] = "1"
    rows = []
    for trial in TRIALS:
        order = MODES if trial % 2 else tuple(reversed(MODES))
        for scene, size in scene_sizes:
            for mode in order:
                rows.append(run_one(binary, out, trial, mode, scene, size, args.timeout))
    summary = report(rows, exe_hash, scene_sizes)
    (out / "summary.md").write_text(summary, encoding="utf-8")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    print(summary, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"AKULA_IMG_PERF_07_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
