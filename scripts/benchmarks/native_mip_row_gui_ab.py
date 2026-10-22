#!/usr/bin/env python3
"""Real Win32 or Linux/Xvfb image A/B. One uninstalled binary, two env modes.

This compares the SCALAR and ROW reducers inside Akula's existing experimental
Vello image mip pipeline, not synthetic FFI calls. It does not claim that
the user's installed app already has image mipmaps enabled.
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
from native_mipmap_windows_ab import percentile, replay_work, stage_samples, windows_scenes

MODES = ("scalar", "row")
TRIALS = (1, 2, 3)


def measured(row: dict) -> dict:
    whole, live = stage_samples(row)
    actual_builds = sum(float(x["mipGenerated"]) for x in whole)
    build_ms = sum(float(x["mipBuildMs"]) for x in whole)
    if actual_builds < 2 or build_ms <= 0:
        raise ValueError("Full app never generated real mip levels")
    return {
        "replay_cpu_p95_ms": percentile(replay_work(row), .95),
        "replay_cpu_p99_ms": percentile(replay_work(row), .99),
        "frame_p95_ms": float(row["metrics"]["frameP95Ms"]),
        "raster_p95_ms": float(row["metrics"]["rasterP95Ms"]),
        "all_frame_mip_build_ms": build_ms,
        "all_frame_mip_generated": int(actual_builds),
        "first_build_frame_ms": next(
            (float(x["mipBuildMs"]) for x in whole if x["mipGenerated"] > 0),
            0.0),
        "peak_rss_kib": row["peak_rss_kib"],
        "whole_frames": len(whole),
        "replay_frames": len(live),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=50.0)
    args = parser.parse_args()

    binary = args.binary.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    if os.name == "nt":
        scene, viewport = windows_scenes()[0]
    else:
        if not os.environ.get("DISPLAY"):
            raise RuntimeError("Linux run requires live Xvfb/X11 DISPLAY")
        scene, viewport = "linux_xvfb", "1000x600"
    if SOURCE.read_bytes()[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("No pinned PNG image")
    fixture_sha = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    binary_sha = hashlib.sha256(binary.read_bytes()).hexdigest()

    rows = []
    for trial in TRIALS:
        order = MODES if trial % 2 else tuple(reversed(MODES))
        for name in order:
            # run_one inherits this variable into a NEW GUI process for each
            # trial, while independently setting all other mip/filter options.
            prior = os.environ.get("AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW")
            try:
                if name == "row":
                    os.environ["AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW"] = "1"
                else:
                    os.environ.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW", None)
                path = f"{scene}-{name}"
                row = run_one(binary, out, trial, "mip_high", path, viewport, args.timeout)
            finally:
                if prior is None:
                    os.environ.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW", None)
                else:
                    os.environ["AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW"] = prior
            log = (out / path / f"trial-{trial}" / "mip_high" / "process.log").read_text(
                errors="replace")
            marker = "AKULA_MIP_HALF_ROW_USED level=" in log
            if marker != (name == "row"):
                raise ValueError(f"{name} trial={trial}: missing/extra row-code witness")
            if not row["applied"] or not row["window_witness"] and os.name == "nt":
                raise ValueError(f"{name} trial={trial}: no installed-window-independent mip witness")
            if row["image_sha256"] != fixture_sha:
                raise ValueError("Different fixture bytes between modes")
            if row["note_sha256"] != rows[0]["note_sha256"] if rows else False:
                raise ValueError("Note bytes changed between trials")
            entry = {
                "mode": name, "trial": trial,
                "scenario": scene, "viewport": viewport,
                "input_image_sha256": fixture_sha,
                "binary_sha256": binary_sha,
                "row_code_witness": marker,
                "metrics": measured(row),
            }
            rows.append(entry)
            print(f"AKULA_MIP_ROW_GUI trial={trial} mode={name} "
                  f"mip_ms={entry['metrics']['all_frame_mip_build_ms']:.3f} "
                  f"replay_p95_ms={entry['metrics']['replay_cpu_p95_ms']:.3f} "
                  f"rss_kib={entry['metrics']['peak_rss_kib']}", flush=True)

    assert len(rows) == len(MODES) * len(TRIALS)
    out.joinpath("matrix.json").write_text(json.dumps(rows, indent=2) + "\n")
    by_trial = {trial: {r["mode"]: r for r in rows if r["trial"] == trial}
                for trial in TRIALS}
    columns = ("all_frame_mip_build_ms", "replay_cpu_p95_ms",
               "frame_p95_ms", "raster_p95_ms", "peak_rss_kib")
    lines = [
        "# Akula real native GUI: baseline mip versus Rust row reducer",
        "",
        f"Platform: {scene}; viewport={viewport}. Same executable SHA256={binary_sha}.",
        f"PNG SHA256={fixture_sha}. Three AB/BA paired trials, six independent GUI processes.",
        "Both modes force existing experimental mip + high-quality image sampling.",
        "Only additional variable is AKULA_EXPERIMENTAL_IMAGE_MIP_HALF_ROW=1 for candidate.",
        "Positive renderer usage marker is required for every candidate process.",
        "Windows processes require uncloaked PID-owned Akula HWND evidence.",
        "Whole-process mip build includes first cold/deferred generation, not just replay.",
        "Replay p95 measures CPU work on drawn replay frames; it is NOT monitor FPS.",
        "No absolute performance gate on shared CI runners; no change to installed binary.",
        "",
        "| Trial | Mode | Mip build total ms | Replay CPU p95 ms | Frame p95 ms | Peak RSS KiB |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for trial in TRIALS:
        for mode in MODES:
            m = by_trial[trial][mode]["metrics"]
            lines.append(f"| {trial} | {mode} | {m['all_frame_mip_build_ms']:.3f} | "
                         f"{m['replay_cpu_p95_ms']:.3f} | {m['frame_p95_ms']:.3f} | "
                         f"{m['peak_rss_kib']} |")
    lines.append("")
    for key in columns:
        pairs = []
        for trial in TRIALS:
            scalar = by_trial[trial]["scalar"]["metrics"][key]
            row = by_trial[trial]["row"]["metrics"][key]
            pairs.append(row - scalar)
        lines.append(f"Paired row - scalar {key}: "
                     f"{[round(value, 4) for value in pairs]}; "
                     f"median={median(pairs):+.4f}.")
    lines.append("")
    lines.append("No production activation until Windows+Linux fidelity/performance evidence is reviewed.")
    summary = "\n".join(lines) + "\n"
    out.joinpath("summary.md").write_text(summary)
    print(summary, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"AKULA_MIP_ROW_GUI_ERROR {type(error).__name__}: {error}",
              file=sys.stderr)
        raise
