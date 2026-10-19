#!/usr/bin/env python3
"""Native Win32/Winit image-scroll A/B: developer build only, not production."""
from __future__ import annotations

import argparse
import ctypes
import hashlib
import json
import os
import sys
from pathlib import Path
from statistics import median

from native_mipmap_window_ab import SOURCE, run_one

TRIALS = (1, 2, 3)
MODES = ("baseline", "mip_medium_linger")


def windows_scenes() -> tuple[tuple[str, str], ...]:
    if os.name != "nt":
        raise RuntimeError("A native Windows GUI runner is required")
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.GetSystemMetrics.argtypes = [ctypes.c_int]
    user32.GetSystemMetrics.restype = ctypes.c_int
    width, height = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    if width < 1000 or height < 700:
        raise ValueError(f"Insufficient interactive desktop: {width}x{height}")
    # Never silently create a 1750px-wide GUI outside a smaller CI desktop.
    compact = (min(1000, int(width * .75)), min(600, int(height * .68)))
    large = (min(1750, int(width * .96)), min(1051, int(height * .88)))
    if large[0] <= compact[0] or large[1] <= compact[1]:
        raise ValueError(f"Non-distinct viewport sizes: {compact}, {large}")
    print(
        f"AKULA_WIN_MIP_DESKTOP monitor={width}x{height} "
        f"compact={compact[0]}x{compact[1]} large={large[0]}x{large[1]}",
        flush=True,
    )
    return (
        ("windows_compact", f"{compact[0]}x{compact[1]}"),
        ("windows_large", f"{large[0]}x{large[1]}"),
    )


def metric(row: dict, name: str) -> float:
    value = row["metrics"].get(name)
    if isinstance(value, bool) or not isinstance(value, (float, int)):
        raise ValueError(f"Missing numeric metric: {name}")
    return float(value)


def report(rows: list[dict], exe_hash: str,
           scenes: tuple[tuple[str, str], ...]) -> str:
    if len(rows) != len(TRIALS) * len(MODES) * len(scenes):
        raise ValueError("Incomplete Windows native GUI trial matrix")
    lines = [
        "# Native Windows image downscale A/B",
        "",
        f"Uninstalled developer release SHA-256: {exe_hash}.",
        "Actual Win32/Winit window, pinned photo and Markdown note, 2 workers.",
        "Three trials per mode/scene, randomized AB/BA process order.",
        "RSS = actual process peak working set; not free system memory.",
        "Frame p95 is submitted CPU work, NOT monitor scanout or visual judder.",
        "No hard latency threshold on shared Windows CI.",
        "Shipping installed builds are tested separately and keep mip OFF.",
        "",
        "| Scene | Viewport | Mode | Frame p95 ms | Raster p95 ms | RSS MiB |",
        "|---|---|---|---:|---:|---:|",
    ]
    for scene, viewport in scenes:
        selected = [row for row in rows if row["scene"] == scene]
        for key in ("note_sha256", "image_sha256"):
            if len({row[key] for row in selected}) != 1:
                raise ValueError(f"Fixture mismatch: {scene}/{key}")
        by_mode = {}
        for mode in MODES:
            group = [row for row in selected if row["mode"] == mode]
            if sorted(row["trial"] for row in group) != list(TRIALS):
                raise ValueError(f"Incomplete {scene}/{mode} trial IDs")
            if not all(row["applied"] == (mode != "baseline") for row in group):
                raise ValueError(f"Mipmap positive/negative witness failed: {scene}/{mode}")
            witnesses = [row.get("window_witness") for row in group]
            if not all(isinstance(w, dict) and w.get("title_akula") is True
                       and w.get("uncloaked") is True
                       and w.get("on_primary_desktop") is True
                       and w.get("width", 0) >= 400 and w.get("height", 0) >= 300
                       for w in witnesses):
                raise ValueError(f"Missing real visible Win32 HWND/PID evidence: {scene}/{mode}")
            by_mode[mode] = group
            frame = median(metric(row, "frameP95Ms") for row in group)
            raster = median(metric(row, "rasterP95Ms") for row in group)
            rss = median(row["peak_rss_kib"] for row in group) / 1024
            frames = [
                float(value)
                for row in group
                for value in row["metrics"]["frameSamplesMs"]
            ]
            if not frames or not all(0 <= value < float("inf") for value in frames):
                raise ValueError(f"Missing/non-finite per-frame times in {scene}/{mode}")
            # p95 alone can conceal a single costly first-time mip generation.
            # These are CPU frame-work samples, NOT physical scanout misses.
            worst = max(frames)
            over_60hz = sum(value > 1000 / 60 for value in frames)
            lines.append(
                f"| {scene} | {viewport} | {mode} | {frame:.3f} | "
                f"{raster:.3f} | {rss:.1f} |"
            )
            lines.append(
                f"{scene}/{mode}: worst CPU frame {worst:.3f} ms; "
                f"work samples > 16.67ms: {over_60hz}/{len(frames)}."
            )
            sizes = [f"{w['width']}x{w['height']}" for w in witnesses]
            lines.append(f"{scene}/{mode}: observed PID-matched Win32 window extents {sizes}.")
        differences = []
        for trial in TRIALS:
            base = next(row for row in by_mode["baseline"] if row["trial"] == trial)
            mip = next(row for row in by_mode["mip_medium_linger"]
                       if row["trial"] == trial)
            differences.append(metric(mip, "frameP95Ms") - metric(base, "frameP95Ms"))
        lines.append(
            f"{scene}: paired p95 delta (mip - original) ms "
            f"{[round(v, 3) for v in differences]}, "
            f"wins={sum(v < 0 for v in differences)}/{len(differences)}."
        )
        for mode in MODES:
            quarter_medians = [
                median(float(row["frame_distribution"]["quarters"][quarter]["frame_p95_ms"])
                       for row in by_mode[mode])
                for quarter in range(4)
            ]
            lines.append(
                f"{scene}/{mode}: quarter p95 ms "
                f"{[round(v, 3) for v in quarter_medians]}."
            )
        levels = [int(row["cache"]["generated"])
                  for row in by_mode["mip_medium_linger"]]
        if not all(levels):
            raise ValueError("Candidate never built mip levels")
        lines.append(
            f"{scene}: mip levels built median={median(levels):.0f}; "
            f"warm hits median={median(int(row['cache']['final_hits']) for row in by_mode['mip_medium_linger']):.0f}."
        )
        lines.append("")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=50)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Refusing to label a non-Windows run as Windows GUI QA")
    binary = args.binary.resolve(strict=True)
    if binary.suffix.lower() != ".exe" or not binary.is_file():
        raise ValueError("Expected native Windows developer release executable")
    png = SOURCE.read_bytes()
    if png[:8] != b"\x89PNG\r\n\x1a\n" or int.from_bytes(png[16:20], "big") < 600:
        raise ValueError("Missing pinned source image")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    scenes = windows_scenes()
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    print(f"AKULA_WIN_MIP_BINARY sha256={digest}", flush=True)
    rows = []
    for trial in TRIALS:
        modes = MODES if trial % 2 else tuple(reversed(MODES))
        for scene, viewport in scenes:
            for mode in modes:
                rows.append(run_one(binary, out, trial, mode, scene, viewport, args.timeout))
    summary = report(rows, digest, scenes)
    (out / "summary.md").write_text(summary, encoding="utf-8")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    print(summary, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"AKULA_WIN_MIP_GUI_ERROR {type(error).__name__}: {error}", file=sys.stderr)
        raise SystemExit(1)
