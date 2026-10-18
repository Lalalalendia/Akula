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
from native_scroll_window_ab import number

REPO = Path(__file__).resolve().parents[2]
SOURCE = REPO / "aquilum-app/native/assets/city-cover.png"
SCENES = (("image_compact", "1000x600"), ("image_large", "1750x1051"))
TRIALS = (1, 2, 3)
MODES = ("baseline", "candidate")


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


def rss_kib(pid: int) -> int:
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
    env["AKULA_VELLO_SINGLE_RECT"] = "1"
    env.pop("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT", None)
    if mode == "candidate":
        env["AKULA_EXPERIMENTAL_IMAGE_MIP"] = "1"
        env["AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF"] = "1"
    else:
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP", None)
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF", None)
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
    if applied != (mode == "candidate"):
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
    result = dict(
        trial=trial, mode=mode, scene=scene, size=size,
        note_sha256=note_hash, image_sha256=image_hash,
        peak_rss_kib=rss, applied=applied, replay=replay, metrics=metrics,
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
        raise ValueError("Incomplete 12-run native window A/B")
    lines = [
        "# Akula real Markdown image scroll A/B",
        "",
        f"Same release executable SHA-256: {binary_hash}.",
        "Real city-cover PNG 1280x884 rendered at 200px, also cropped.",
        "Winit/Masonry/softbuffer on Linux Xvfb/Openbox, 2 workers.",
        "Same input bytes; three fresh process pairs in alternating order.",
        "Candidate requires affirmative native mip witness; baseline must not have it.",
        "Timings and RSS are diagnostic, not a user-visible physical FPS measurement.",
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
        old, new = values["baseline"], values["candidate"]
        lines.append(
            f"\n{scene}: candidate CPU work {(new[0] / old[0] - 1)*100:+.2f}%, "
            f"sampled RSS {(new[3] / old[3] - 1)*100:+.2f}% vs baseline.\n"
        )
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
        order = MODES if trial % 2 else tuple(reversed(MODES))
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
