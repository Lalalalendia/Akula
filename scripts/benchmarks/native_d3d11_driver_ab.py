#!/usr/bin/env python3
"""Strict developer-only Windows D3D11 driver A/B using real Win32 Akula windows.

This measures CPU work + peak process RSS, NOT GPU rendering or physical scanout.
The driver proof is emitted by Rust only after successful device/swap-chain creation.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path
from statistics import median

from native_mipmap_window_ab import run_one
from native_mipmap_windows_ab import percentile, replay_work, windows_scenes

MODES = ("warp", "hardware")
TRIALS = (1, 2, 3)


def validate_proof(path: Path, driver: str) -> None:
    if not path.is_file() or path.read_text(encoding="utf-8") != f"driver={driver}\n":
        raise ValueError(f"Missing positive D3D11 {driver} driver witness: {path}")


def summarize(rows: list[dict], binary_hash: str) -> str:
    if len(rows) != len(TRIALS) * len(MODES) * 2:
        raise ValueError("Incomplete strict 12-process driver matrix")
    out = [
        "# Akula D3D11 WARP / hardware strict Windows A/B", "",
        f"Same uninstalled release binary SHA256: {binary_hash}.",
        "Forced driver must create the actual D3D11 device, swap chain and canvas;",
        "NO fallback when forced. Both use the same CPU Vello raster pipeline.",
        "Independent real Win32 GUI processes, alternating AB/BA order, two viewports.",
        "Metrics are CPU-submitted replay work and process peak RSS, not GPU timings,",
        "screen scanout, wattage, or subjective smoothness. No default driver changes.",
        "", "| Scene | Driver | Replay CPU p95 (ms) | p99 (ms) | Peak RSS (MiB) |",
        "|---|---|---:|---:|---:|",
    ]
    for scene in ("windows_compact", "windows_large"):
        cases = [row for row in rows if row["scene"] == scene]
        if len({row["note_sha256"] for row in cases}) != 1 or len({row["image_sha256"] for row in cases}) != 1:
            raise ValueError(f"Nonidentical fixture: {scene}")
        for mode in MODES:
            sample = [row for row in cases if row["driver"] == mode]
            if sorted(row["trial"] for row in sample) != list(TRIALS):
                raise ValueError(f"Incomplete trials: {scene}/{mode}")
            if not all(row["window_witness"] and row["window_witness"]["title_akula"] for row in sample):
                raise ValueError(f"Missing real Win32 window: {scene}/{mode}")
            p95 = median(percentile(replay_work(row), .95) for row in sample)
            p99 = median(percentile(replay_work(row), .99) for row in sample)
            rss = median(row["peak_rss_kib"] for row in sample) / 1024
            out.append(f"| {scene} | {mode} | {p95:.3f} | {p99:.3f} | {rss:.1f} |")
    return "\n".join(out) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=50)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Windows native GUI required")
    binary = args.binary.resolve(strict=True)
    if binary.suffix.lower() != ".exe":
        raise ValueError("Expected uninstalled Windows .exe")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    scenes = windows_scenes()
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    rows = []
    for trial in TRIALS:
        modes = MODES if trial % 2 else tuple(reversed(MODES))
        for scene, size in scenes:
            for mode in modes:
                proof = out / scene / f"trial-{trial}" / mode / "driver-proof.txt"
                proof.parent.mkdir(parents=True, exist_ok=True)
                proof.unlink(missing_ok=True)
                os.environ["AKULA_EXPERIMENTAL_D3D11_DRIVER"] = mode
                os.environ["AKULA_EXPERIMENTAL_D3D11_PROOF"] = str(proof)
                try:
                    # mode=baseline keeps mipmaps OFF for both D3D11 routes.
                    row = run_one(binary, out / mode, trial, "baseline", scene, size, args.timeout)
                finally:
                    os.environ.pop("AKULA_EXPERIMENTAL_D3D11_DRIVER", None)
                    os.environ.pop("AKULA_EXPERIMENTAL_D3D11_PROOF", None)
                validate_proof(proof, mode)
                row["driver"] = mode
                rows.append(row)
    summary = summarize(rows, digest)
    (out / "summary.md").write_text(summary, encoding="utf-8")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    print(summary, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"AKULA_D3D11_AB_ERROR {type(error).__name__}: {error}", file=sys.stderr)
        raise SystemExit(1)
