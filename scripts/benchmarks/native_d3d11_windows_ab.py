#!/usr/bin/env python3
"""Windows developer-only diagnostic: real GUI, exact D3D11 backend, comparable scroll replays."""

import argparse
import hashlib
import json
import os
import pathlib
import statistics
import subprocess
import sys
import time

# Reuse the separately exercised Win32 GUI/PID and working-set probes.
from native_mipmap_window_ab import windows_gui_witness, windows_peak_rss_kib
from native_scroll_window_ab import number

SCENE = "# Windows D3D11 probe\n\n" + ("Paragraph **bold** text and some кириллица.\n\n" * 350)
MODES = ("warp", "hardware")
TRIALS = (1, 2)


def run(binary, out, trial, mode):
    root = out / f"trial-{trial}" / mode
    vault = root / "vault"
    data = root / "data"
    vault.mkdir(parents=True, exist_ok=True)
    data.mkdir(parents=True, exist_ok=True)
    note = vault / "scroll.md"
    note.write_text(SCENE, encoding="utf-8")
    source_hash = hashlib.sha256(note.read_bytes()).hexdigest()

    metrics = root / "metrics.json"
    replay = root / "replay.json"
    log = root / "process.log"
    env = dict(os.environ, AKULA_EXPERIMENTAL_D3D11=mode, AKULA_EXPERIMENTAL_FRAME_SAMPLES="1")
    cmd = [str(binary), "--vault", str(vault), "--data-dir", str(data),
           "--open", str(note), "--threads", "2", "--size", "1200x760",
           "--replay-scroll", str(replay), "--metrics", str(metrics)]

    # A JSON report alone is not evidence of a real, visible native window.
    # Verify the same process owns an uncloaked Win32 HWND before it exits.
    seen = None
    peak_rss_kib = 0
    with log.open("wb") as stream:
        child = subprocess.Popen(cmd, env=env, stdout=stream, stderr=subprocess.STDOUT)
        started = time.monotonic()
        try:
            while child.poll() is None:
                seen = windows_gui_witness(child.pid) or seen
                peak_rss_kib = max(peak_rss_kib, windows_peak_rss_kib(child.pid))
                if time.monotonic() - started > 75.0:
                    raise TimeoutError(f"native GUI did not exit: {trial}/{mode}")
                time.sleep(0.05)
            if child.returncode != 0:
                raise RuntimeError(f"native GUI failed rc={child.returncode}: {trial}/{mode}; see {log}")
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)
    if seen is None or not seen.get("uncloaked") or not seen.get("title_akula"):
        raise RuntimeError(f"no visible non-cloaked Akula HWND for {trial}/{mode}; see {log}")
    if peak_rss_kib <= 0:
        raise RuntimeError(f"no Win32 process memory samples for {trial}/{mode}")

    # A strict device probe must never silently measure the other driver.
    marker = f"AKULA_D3D11_PROBE requested={mode} actual={mode} strict=1"
    raw = log.read_text(encoding="utf-8", errors="replace")
    if raw.count(marker) < 1:
        raise RuntimeError(f"missing exact backend witness for {trial}/{mode}: {log}")
    replay_data = json.loads(replay.read_text(encoding="utf-8"))
    metrics_data = json.loads(metrics.read_text(encoding="utf-8"))
    frames = int(number(replay_data, "frames", 20))
    if number(metrics_data, "frames", 20) < frames:
        raise ValueError(f"inconsistent real GUI frame counts for {trial}/{mode}")
    sample = {
        "trial": trial,
        "mode": mode,
        "note_sha256": source_hash,
        "frames": frames,
        "work_avg_ms": number(replay_data, "workAvgMs", 0.00001),
        "interval_p95_ms": number(replay_data, "intervalP95Ms", 0.00001),
        "frame_p95_ms": number(metrics_data, "frameP95Ms", 0.00001),
        "raster_p95_ms": number(metrics_data, "rasterP95Ms", 0.00001),
        "peak_rss_kib": peak_rss_kib,
        "window_witness": seen,
        "driver_witness": marker,
    }
    (root / "sample.json").write_text(json.dumps(sample, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"D3D11_GUI_WITNESS trial={trial} mode={mode} frames={frames} rss_kib={peak_rss_kib}", flush=True)
    return sample


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Windows real GUI required: other OS may not run this comparison")
    binary = args.binary.resolve(strict=True)
    if binary.suffix.lower() != ".exe":
        raise ValueError("Expected a native Windows developer .exe")
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)

    rows = []
    for trial in TRIALS:
        order = MODES if trial % 2 else tuple(reversed(MODES))
        for mode in order:
            rows.append(run(binary, out, trial, mode))
    if len(rows) != len(TRIALS) * len(MODES) or len({r["note_sha256"] for r in rows}) != 1:
        raise ValueError("Incomplete or non-identical A/B fixture")
    summary = [
        "# Windows D3D11 WARP vs HARDWARE (diagnostic only)", "",
        "Two independent alternating pairs. Each real native GUI process has an exact backend",
        "witness, visible uncloaked Akula HWND/PID, and measured peak working set.",
        "Rendering remains CPU (vello_cpu). CI VM measurements are not physical GPU/monitor",
        "benchmarks and cannot justify a production driver default on their own.", "",
        "| Trial | Backend | Frames | Work avg ms | Interval p95 ms | Host frame p95 ms | RSS peak MiB |",
        "|---|---|---:|---:|---:|---:|---:|",
    ]
    for row in rows:
        summary.append(
            f"| {row['trial']} | {row['mode']} | {row['frames']} | {row['work_avg_ms']:.2f} | "
            f"{row['interval_p95_ms']:.2f} | {row['frame_p95_ms']:.2f} | "
            f"{row['peak_rss_kib']/1024:.1f} |"
        )
    summary.append("")
    for key in ("work_avg_ms", "interval_p95_ms", "frame_p95_ms", "raster_p95_ms", "peak_rss_kib"):
        summary.append(
            f"{key}: median WARP={statistics.median(row[key] for row in rows if row['mode']=='warp'):.3f}, "
            f"HARDWARE={statistics.median(row[key] for row in rows if row['mode']=='hardware'):.3f}."
        )
    (out / "matrix.json").write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (out / "summary.md").write_text("\n".join(summary) + "\n", encoding="utf-8")
    print("\n".join(summary))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"AKULA_D3D11_AB_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        raise
