#!/usr/bin/env python3
"""Windows-only diagnostic: one native GUI process per explicitly selected D3D11 driver."""
import argparse
import json
import os
import pathlib
import statistics
import subprocess
import sys

def run(binary, out, trial, mode):
    root = out / f"trial-{trial}" / mode
    vault = root / "vault"
    data = root / "data"
    vault.mkdir(parents=True)
    data.mkdir()
    note = vault / "scroll.md"
    note.write_text("# Windows D3D11 probe\n\n" + ("Paragraph **bold** text and some кириллица.\n\n" * 350), encoding="utf-8")
    metrics = root / "metrics.json"
    replay = root / "replay.json"
    log = root / "process.log"
    env = dict(os.environ, AKULA_EXPERIMENTAL_D3D11=mode, AKULA_EXPERIMENTAL_FRAME_SAMPLES="1")
    cmd = [str(binary), "--vault", str(vault), "--data-dir", str(data),
           "--open", str(note), "--threads", "2", "--size", "1200x760",
           "--replay-scroll", str(replay), "--metrics", str(metrics)]
    with log.open("w", encoding="utf-8") as stream:
        subprocess.run(cmd, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=75, check=True)
    witness = f"AKULA_D3D11_PROBE requested={mode} actual={mode} strict=1"
    raw = log.read_text(encoding="utf-8")
    if raw.count(witness) < 1:
        raise RuntimeError(f"missing exact backend witness for {trial}/{mode}: {log}")
    replay_data = json.loads(replay.read_text(encoding="utf-8"))
    metrics_data = json.loads(metrics.read_text(encoding="utf-8"))
    if replay_data.get("frames", 0) < 20 or metrics_data.get("frames", 0) < replay_data["frames"]:
        raise RuntimeError(f"insufficient real frames for {trial}/{mode}")
    return {"trial": trial, "mode": mode, "frames": replay_data["frames"],
            "work_avg_ms": replay_data["workAvgMs"],
            "interval_p95_ms": replay_data["intervalP95Ms"],
            "frame_p95_ms": metrics_data["frameP95Ms"],
            "raster_p95_ms": metrics_data["rasterP95Ms"],
            "driver_witness": witness}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Windows real GUI required")
    binary = args.binary.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for trial in (1, 2):
        order = ("warp", "hardware") if trial == 1 else ("hardware", "warp")
        for mode in order:
            rows.append(run(binary, out, trial, mode))
    summary = ["# Windows D3D11 WARP vs HARDWARE (diagnostic only)", "",
               "Actual device backend verified in each process. The renderer still rasterizes with vello_cpu on CPU.",
               "Results are CI VM observations, not physical-device FPS or proof of superiority.", "",
               "| Trial | Backend | Frames | Replay work avg ms | Interval p95 ms | Host frame p95 ms |",
               "|---|---|---:|---:|---:|---:|"]
    for row in rows:
        summary.append(f"| {row['trial']} | {row['mode']} | {row['frames']} | {row['work_avg_ms']:.2f} | {row['interval_p95_ms']:.2f} | {row['frame_p95_ms']:.2f} |")
    summary.append("")
    for key in ("work_avg_ms", "interval_p95_ms", "frame_p95_ms"):
        summary.append(f"{key}: median WARP={statistics.median(row[key] for row in rows if row['mode']=='warp'):.3f}, "
                       f"HARDWARE={statistics.median(row[key] for row in rows if row['mode']=='hardware'):.3f}.")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    (out / "summary.md").write_text("\n".join(summary) + "\n", encoding="utf-8")
    print("\n".join(summary))
if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"AKULA_D3D11_AB_ERROR {exc}", file=sys.stderr)
        raise
