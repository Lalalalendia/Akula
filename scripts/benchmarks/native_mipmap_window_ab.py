#!/usr/bin/env python3
"""Real Winit/Masonry/softbuffer image-scroll A/B with single-binary opt-in.

Tests real Markdown PNG embeds, native mip witness, per-process RSS, and
submitted-frame work. Does not measure physical monitor scanout or GPU FPS.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path
from statistics import median
from math import floor, isfinite
from native_scroll_window_ab import number

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO / "aquilum-app/native/assets/city-cover.png"
SCENES = (("image_compact", "1000x600"), ("image_large", "1750x1051"))
TRIALS = (1, 2, 3, 4)
MODES = ("baseline", "mip_high", "mip_medium", "mip_medium_linger")


def p95(samples: list[float]) -> float:
    if not samples or not all(isfinite(s) and s >= 0 for s in samples):
        raise ValueError("invalid frame time samples")
    ordered = sorted(samples)
    return ordered[floor((len(ordered) - 1) * 0.95 + 0.5)]


def frame_distribution(metrics: dict) -> dict:
    frames = metrics.get("frameSamplesMs")
    raster = metrics.get("rasterSamplesMs")
    if not isinstance(frames, list) or not isinstance(raster, list):
        raise ValueError("missing required opt-in per-frame data")
    if len(frames) != len(raster) or len(frames) != int(metrics["frames"]) - 1:
        raise ValueError("incorrect frame/raster series length")
    if len(frames) < 80:
        raise ValueError("too few independently timed frames")
    chunks = []
    for quarter in range(4):
        lo = quarter * len(frames) // 4
        hi = (quarter + 1) * len(frames) // 4
        chunks.append({
            "quarter": quarter + 1,
            "frame_p95_ms": p95(frames[lo:hi]),
            "raster_p95_ms": p95(raster[lo:hi]),
        })
    return {
        "frames_sampled": len(frames),
        "frame_p95_first_half_ms": p95(frames[:len(frames) // 2]),
        "frame_p95_second_half_ms": p95(frames[len(frames) // 2:]),
        "quarters": chunks,
    }


def fixture() -> str:
    header = (
        "# Markdown image downscale A/B\n\n"
        "![200](city-cover.png)\n\n"
        "![200|crop=10,10,90,90](city-cover.png)\n\n"
    )
    body = "".join(
        f"## Section {i}\n"
        "Markdown, **bold**, *italic*, unicode, [[Akula]] and wrapped text.\n\n"
        + ("![200](city-cover.png)\n\n" if i % 12 == 0 else "")
        + ("![200|crop=10,10,90,90](city-cover.png)\n\n" if i % 24 == 0 else "")
        for i in range(240)
    )
    return header + body


# Measure the peak working set on Windows, equivalent in meaning to the
# Linux process VmHWM counter. No fallback-to-zero is allowed by the A/B.
if os.name == "nt":
    import ctypes
    from ctypes import wintypes

    class _ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    _kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _psapi = ctypes.WinDLL("psapi", use_last_error=True)
    _open_process = _kernel32.OpenProcess
    _open_process.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    _open_process.restype = wintypes.HANDLE
    _close_handle = _kernel32.CloseHandle
    _close_handle.argtypes = [wintypes.HANDLE]
    _close_handle.restype = wintypes.BOOL
    _get_memory = _psapi.GetProcessMemoryInfo
    _get_memory.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(_ProcessMemoryCounters), wintypes.DWORD,
    ]
    _get_memory.restype = wintypes.BOOL


def windows_peak_rss_kib(pid: int) -> int:
    if os.name != "nt":
        raise RuntimeError("Windows process memory query requires Windows")
    handle = _open_process(0x0400 | 0x0010, False, pid)
    if not handle:
        return 0  # Process may exit between poll() and OpenProcess.
    try:
        counters = _ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not _get_memory(handle, ctypes.byref(counters), counters.cb):
            return 0
        return int(counters.PeakWorkingSetSize // 1024)
    finally:
        _close_handle(handle)


def rss_kib(pid: int) -> int:
    if os.name == "nt":
        return windows_peak_rss_kib(pid)
    try:
        content = Path(f"/proc/{pid}/status").read_text()
    except OSError:
        return 0
    for field in ("VmHWM:", "VmRSS:"):
        for line in content.splitlines():
            if line.startswith(field):
                return int(line.split()[1])
    return 0


def run_one(binary: Path, out: Path, trial: int, mode: str,
            scene: str, size: str, timeout: float) -> dict:
    root = out / scene / f"trial-{trial}" / mode
    vault, data = root / "vault", root / "data"
    vault.mkdir(parents=True, exist_ok=True)
    data.mkdir(parents=True, exist_ok=True)
    note = vault / "images.md"
    source = fixture()
    note.write_text(source, encoding="utf-8")
    shutil.copyfile(SOURCE, vault / "city-cover.png")
    note_hash = hashlib.sha256(source.encode()).hexdigest()
    image_hash = hashlib.sha256(SOURCE.read_bytes()).hexdigest()
    replay_file = root / "replay.json"
    metrics_file = root / "metrics.json"
    log_file = root / "process.log"
    command = [
        str(binary), "--vault", str(vault), "--data-dir", str(data),
        "--open", str(note), "--threads", "2", "--size", size,
        "--replay-scroll", str(replay_file), "--metrics", str(metrics_file),
    ]
    env = os.environ.copy()
    env["AKULA_EXPERIMENTAL_FRAME_SAMPLES"] = "1"
    env["AKULA_VELLO_SINGLE_RECT"] = "1"
    env.pop("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT", None)
    if mode != "baseline":
        env["AKULA_EXPERIMENTAL_IMAGE_MIP"] = "1"
        env["AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF"] = "1"
        if mode in ("mip_medium", "mip_medium_linger"):
            env["AKULA_EXPERIMENTAL_IMAGE_MIP_FILTER"] = "medium"
        else:
            env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_FILTER", None)
        if mode == "mip_medium_linger":
            env["AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER"] = "1"
        else:
            env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER", None)
    else:
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP", None)
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF", None)
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_FILTER", None)
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER", None)
    rss = 0
    with log_file.open("wb") as log:
        process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, env=env)
        started = time.monotonic()
        try:
            while process.poll() is None:
                rss = max(rss, rss_kib(process.pid))
                if time.monotonic() - started > timeout:
                    raise TimeoutError(f"{scene}/{trial}/{mode}: timed out")
                time.sleep(0.05)
            if process.returncode:
                raise RuntimeError(f"{scene}/{trial}/{mode}: exit={process.returncode}: {log_file}")
        except BaseException:
            process.kill()
            process.wait()
            raise
    if rss <= 0:
        raise ValueError(f"{scene}/{trial}/{mode}: missing process RSS")
    log = log_file.read_text(errors="replace")
    applied = "AKULA_MIP_APPLIED level=" in log
    if applied != (mode != "baseline"):
        raise ValueError(
            f"{scene}/{trial}/{mode}: mip path witness mismatch, "
            f"applied={applied}; see {log_file}"
        )

    replay = json.loads(replay_file.read_text())
    metrics = json.loads(metrics_file.read_text())
    frames = number(replay, "frames", 20)
    if len(replay.get("quarters", [])) != 4:
        raise ValueError(f"{scene}/{mode}: missing replay quarters")
    for key in ("workAvgMs", "fps", "intervalP95Ms"):
        number(replay, key, 0.00001)
    for key in ("frameP95Ms", "rasterP95Ms"):
        number(metrics, key, 0.00001)
    if number(metrics, "threads") != 2 or number(metrics, "frames") < frames:
        raise ValueError(f"{scene}/{mode}: invalid host metrics")
    distribution = frame_distribution(metrics)
    # The host computes p95 on the same samples; tolerate only rounding
    # (summary is written to 2 decimal places).
    if abs(p95(metrics["frameSamplesMs"]) - metrics["frameP95Ms"]) > 0.06:
        raise ValueError(f"{scene}/{mode}: raw frame sample p95 disagrees with host")
    if abs(p95(metrics["rasterSamplesMs"]) - metrics["rasterP95Ms"]) > 0.06:
        raise ValueError(f"{scene}/{mode}: raw raster sample p95 disagrees with host")
    import re
    samples = [dict(pair.split("=", 1) for pair in line.split()[2:] if "=" in pair)
               for line in log.splitlines() if line.startswith("AKULA_MIP_CACHE ")]
    diagnostics = samples[-1] if samples else {}
    if mode != "baseline":
        if len(samples) < 2 or int(diagnostics.get("final_hits", 0)) == 0:
            raise ValueError(f"{scene}/{trial}/{mode}: missing warm-cache diagnostics")
    else:
        if samples:
            raise ValueError(f"{scene}/{trial}/{mode}: baseline unexpectedly reported mip cache")
    result = dict(
        trial=trial, mode=mode, scene=scene, size=size,
        note_sha256=note_hash, image_sha256=image_hash,
        peak_rss_kib=rss, applied=applied, replay=replay, metrics=metrics,
        frame_distribution=distribution, cache=diagnostics,
    )
    (root / "sample.json").write_text(json.dumps(result, indent=2) + "\n")
    print(
        f"AKULA_MIP_GUI scene={scene} trial={trial} mode={mode} "
        f"frames={frames:.0f} work_avg_ms={replay['workAvgMs']:.3f} "
        f"frame_p95_ms={metrics['frameP95Ms']:.3f} "
        f"raster_p95_ms={metrics['rasterP95Ms']:.3f} "
        f"peak_rss_kib={rss} mip_applied={applied}", flush=True,
    )
    return result


def report(rows: list[dict], binary_hash: str) -> str:
    if len(rows) != len(TRIALS) * len(SCENES) * len(MODES):
        raise ValueError("Incomplete 32-run native window A/B")
    lines = [
        "# Akula real Markdown image scroll A/B",
        "",
        f"Same release executable SHA-256: {binary_hash}.",
        "Real city-cover PNG 1280x884 rendered at 200px, also cropped.",
        "Winit/Masonry/softbuffer on Linux Xvfb/Openbox, 2 workers.",
        "Same input bytes; four fresh runs per mode in rotating order.",
        "Modes: original High, mip+High, mip+Medium, mip+Medium+512-frame LRU.",
        "All mip modes require affirmative native witness; baseline must not.",
        "Timings and RSS are diagnostic, not a user-visible physical FPS measurement.",
        "Per-frame raster/total samples are developer-only; all modes measured identically.",
        "",
        "| Scene | Mode | Work avg (ms) | Raster p95 (ms) | Frame p95 (ms) | Peak RSS (MiB) |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for scene, _ in SCENES:
        selected = [r for r in rows if r["scene"] == scene]
        for attr in ("note_sha256", "image_sha256"):
            if len({r[attr] for r in selected}) != 1:
                raise ValueError(f"Fixture hash mismatch: {scene}/{attr}")
        values = {}
        for mode in MODES:
            chunk = [r for r in selected if r["mode"] == mode]
            if sorted(r["trial"] for r in chunk) != list(TRIALS):
                raise ValueError(f"Missing trials: {scene}/{mode}")
            work = median(float(r["replay"]["workAvgMs"]) for r in chunk)
            raster = median(float(r["metrics"]["rasterP95Ms"]) for r in chunk)
            frame = median(float(r["metrics"]["frameP95Ms"]) for r in chunk)
            memory = median(r["peak_rss_kib"] for r in chunk) / 1024
            values[mode] = (work, raster, frame, memory)
            lines.append(
                f"| {scene} | {mode} | {work:.3f} | {raster:.3f} | "
                f"{frame:.3f} | {memory:.1f} |"
            )
        old = values["baseline"]
        for mode in MODES[1:]:
            new = values[mode]
            lines.append(
                f"\n{scene}/{mode}: CPU work {(new[0] / old[0] - 1)*100:+.2f}%, "
                f"frame p95 {(new[2] / old[2] - 1)*100:+.2f}%, "
                f"sampled RSS {(new[3] / old[3] - 1)*100:+.2f}% vs baseline."
            )
        high, medium = values["mip_high"], values["mip_medium"]
        lines.append(
            f"\n{scene}: bilinear vs mipped bicubic frame p95 "
            f"{(medium[2] / high[2] - 1)*100:+.2f}%; "
            f"raster p95 {(medium[1] / high[1] - 1)*100:+.2f}%."
        )
        for mode in MODES[1:]:
            cache = [r["cache"] for r in selected if r["mode"] == mode]
            lines.append(
                f"{scene}/{mode}: median cache final_hits="
                f"{median(int(x['final_hits']) for x in cache):.0f}, "
                f"generated_levels={median(int(x['generated']) for x in cache):.0f}, "
                f"generated_ms={median(float(x['generated_ms']) for x in cache):.2f}."
            )
        # Pair each candidate process with its baseline from the same trial.
        # A median of separately aggregated p95 values can hide noisy runs.
        pairs = []
        for trial in TRIALS:
            trial_rows = {r["mode"]: r for r in selected if r["trial"] == trial}
            delta = (
                trial_rows["mip_medium_linger"]["metrics"]["frameP95Ms"]
                - trial_rows["baseline"]["metrics"]["frameP95Ms"]
            )
            pairs.append(delta)
        lines.append(
            f"{scene}: paired 512-frame vs baseline frame-p95 differences (ms) "
            f"={[round(d, 3) for d in pairs]}, "
            f"wins={sum(d < 0 for d in pairs)}/{len(pairs)}, "
            f"median_delta_ms={median(pairs):+.3f}."
        )
        for mode in ("baseline", "mip_medium_linger"):
            sub = [r for r in selected if r["mode"] == mode]
            quarter_medians = [
                median(r["frame_distribution"]["quarters"][q]["frame_p95_ms"] for r in sub)
                for q in range(4)
            ]
            early = median(r["frame_distribution"]["frame_p95_first_half_ms"] for r in sub)
            late = median(r["frame_distribution"]["frame_p95_second_half_ms"] for r in sub)
            lines.append(
                f"{scene}/{mode}: quarter frame-p95 (ms) "
                f"{[round(t, 3) for t in quarter_medians]}, "
                f"first_half={early:.3f}, second_half={late:.3f}."
            )
        linger = values["mip_medium_linger"]
        lines.append(
            f"{scene}: 512-frame retention vs two-frame retention "
            f"frame p95 {(linger[2] / medium[2] - 1)*100:+.2f}%, "
            f"raster p95 {(linger[1] / medium[1] - 1)*100:+.2f}%, "
            f"sampled RSS {(linger[3] / medium[3] - 1)*100:+.2f}%."
        )
        lines.append("")
    lines.append(
        "No timing threshold: this Xvfb experiment cannot verify physical "
        "frame smoothness and does not replace photo/alpha pixel fidelity gates."
    )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=40)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    data = SOURCE.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Benchmark fixture must be the repository PNG")
    width = int.from_bytes(data[16:20], "big")
    height = int.from_bytes(data[20:24], "big")
    if width < 600 or height < 400:
        raise ValueError(f"Unexpectedly small image {width}x{height}")
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    print(f"AKULA_MIP_BINARY sha256={digest} source_px={width}x{height}")
    rows = []
    for trial in TRIALS:
        order = MODES[trial - 1:] + MODES[:trial - 1]
        for scene, size in SCENES:
            for mode in order:
                rows.append(run_one(binary, out, trial, mode, scene, size, args.timeout))
    summary = report(rows, digest)
    (out / "summary.md").write_text(summary)
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(summary)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"AKULA_MIP_GUI_ERROR {type(error).__name__}: {error}", file=sys.stderr)
        raise SystemExit(1)
