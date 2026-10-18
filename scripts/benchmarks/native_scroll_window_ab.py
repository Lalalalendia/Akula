#!/usr/bin/env python3
"""Real Akula Winit/softbuffer scroll replay A/B: no synthetic render harness.

Runs the existing --replay-scroll and --metrics flags in independent GUI
processes. Linux CI uses Xvfb; a local desktop can run the same command.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from statistics import median

SCENES = (
    ("plain_small", "1000x600", False),
    ("cover_large", "1750x1051", True),
)
TRIALS = (1, 2, 3)
MODES = ("fixed2", "adaptive2")


def fixture(cover: bool, emoji_colrv1: bool = False) -> str:
    header = "---\ncover: true\ntype: book\nPage_cover: pattern:polka\n---\n" if cover else ""
    emoji = "😁 😃 😄 😅 😆 😇 😊 😍 😎 🙂 " * 8 if emoji_colrv1 else ""
    paragraphs = "".join(
        f"## Section {n}\n"
        f"Text {n}: Akula scrolling Unicode and кириллица. "
        "Some **bold** and *italic* Markdown, [[Akula]] links, "
        "and a long sentence that must wrap across the editor column. "
        f"The second line keeps the document scrollable.{(' ' + emoji) if emoji else ''}\n\n"
        for n in range(300)
    )
    return header + paragraphs


def number(obj: dict, key: str, minimum: float = 0.0) -> float:
    value = obj.get(key)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"Non-numeric/missing {key}: {value!r}")
    value = float(value)
    if not (minimum <= value < 1e9):
        raise ValueError(f"Invalid {key}: {value}")
    return value


def run_one(binary: Path, out: Path, trial: int, mode: str, scene: str,
            size: str, cover: bool, timeout: float) -> dict:
    root = out / scene / f"trial-{trial}" / mode
    vault, data = root / "vault", root / "data"
    vault.mkdir(parents=True, exist_ok=True)
    data.mkdir(parents=True, exist_ok=True)
    note = vault / "scroll-fixture.md"
    content = fixture(cover, emoji_colrv1=scene.startswith("colr_"))
    note.write_text(content, encoding="utf-8")
    source_digest = hashlib.sha256(content.encode("utf-8")).hexdigest()
    replay_file, metrics_file, log = (root / "replay.json", root / "metrics.json", root / "process.log")
    route = "0" if mode == "fixed2" else "1"
    command = [
        str(binary), "--vault", str(vault), "--data-dir", str(data),
        "--open", str(note), "--threads", "2", "--size", size,
        "--replay-scroll", str(replay_file), "--metrics", str(metrics_file),
    ]
    env = os.environ.copy()
    env["AKULA_VELLO_SINGLE_RECT"] = route
    env.pop("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT", None)

    started = time.monotonic()
    with log.open("wb") as stream:
        try:
            subprocess.run(
                command, stdout=stream, stderr=subprocess.STDOUT,
                env=env, timeout=timeout, check=True,
            )
        except (subprocess.TimeoutExpired, subprocess.CalledProcessError) as exc:
            raise RuntimeError(f"{scene}/{trial}/{mode} failed: {exc}; read {log}") from exc
    elapsed = time.monotonic() - started
    try:
        replay = json.loads(replay_file.read_text(encoding="utf-8"))
        metrics = json.loads(metrics_file.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise RuntimeError(f"{scene}/{trial}/{mode} produced no valid GUI metrics; see {log}") from exc

    frames = int(number(replay, "frames", 1))
    if frames < 20:
        raise ValueError(f"{scene}/{trial}/{mode}: only {frames} nonempty replay frames")
    if len(replay.get("quarters", [])) != 4:
        raise ValueError(f"{scene}/{trial}/{mode}: replay does not cover all four quarters")
    fields = ("fps", "intervalAvgMs", "intervalP95Ms", "workAvgMs")
    for key in fields:
        number(replay, key, 0.00001)
    for key in ("frameAvgMs", "frameP95Ms", "rasterAvgMs", "rasterP95Ms"):
        number(metrics, key, 0.00001)
    if int(number(metrics, "threads")) != 2:
        raise ValueError(f"{scene}/{trial}/{mode}: wrong CLI worker count")
    if int(number(metrics, "frames")) < frames:
        raise ValueError(f"{scene}/{trial}/{mode}: inconsistent host/replay frame counts")
    number(metrics, "scaleFactor", 0.1)
    result = {
        "trial": trial, "mode": mode, "scene": scene, "size": size,
        "source_sha256": source_digest, "elapsed_s": round(elapsed, 3),
        "replay": replay, "metrics": metrics,
    }
    (root / "sample.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(
        f"AKULA_WINDOW_SCROLL scene={scene} trial={trial} mode={mode} "
        f"frames={frames} fps={replay['fps']:.1f} "
        f"interval_p95_ms={replay['intervalP95Ms']:.2f} "
        f"work_avg_ms={replay['workAvgMs']:.2f} "
        f"frame_p95_ms={metrics['frameP95Ms']:.2f} "
        f"raster_p95_ms={metrics['rasterP95Ms']:.2f}",
        flush=True,
    )
    return result


def summarize(out: Path, rows: list[dict]) -> str:
    if len(rows) != len(SCENES) * len(TRIALS) * len(MODES):
        raise ValueError("Incomplete independent GUI A/B matrix")
    lines = [
        "# Akula: existing real-window scroll replay A/B",
        "",
        "One release-built native Akula window per process; same source fixture",
        "and 2 raster workers; separate data directories and alternating order.",
        "Linux CI uses Winit + Masonry + softbuffer under **Xvfb/Openbox**.",
        "Replay records only nonempty-damage frame submissions. This tests",
        "the window present path, **not** monitor scanout, human-visible FPS,",
        "physical display-vsync deadlines, or latency of a physical pointer.",
        "The built-in missed counter assumes a 120 Hz reference even in Xvfb;",
        "it is recorded but NOT treated as dropped physical display frames.",
        "",
        "| Scene | Policy | Frames (median) | Submitted FPS (median) | "
        "Interval p95 (ms) | Work avg (ms) | Host frame p95 (ms) | Host raster p95 (ms) |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for scene, _, cover in SCENES:
        selected = [r for r in rows if r["scene"] == scene]
        digests = {r["source_sha256"] for r in selected}
        if len(digests) != 1:
            raise ValueError(f"Different source note data in {scene}")
        by_mode = {}
        for mode in MODES:
            subset = [r for r in selected if r["mode"] == mode]
            if sorted(r["trial"] for r in subset) != list(TRIALS):
                raise ValueError(f"Missing independent run in {scene}/{mode}")
            get = lambda name, section: median(float(r[section][name]) for r in subset)
            by_mode[mode] = {
                "frames": get("frames", "replay"),
                "fps": get("fps", "replay"),
                "p95": get("intervalP95Ms", "replay"),
                "work": get("workAvgMs", "replay"),
                "frame_p95": get("frameP95Ms", "metrics"),
                "raster_p95": get("rasterP95Ms", "metrics"),
            }
            m = by_mode[mode]
            lines.append(
                f"| {scene} | {mode} | {m['frames']:.0f} | {m['fps']:.1f} | "
                f"{m['p95']:.2f} | {m['work']:.2f} | "
                f"{m['frame_p95']:.2f} | {m['raster_p95']:.2f} |"
            )
        fixed, adaptive = (by_mode[mode] for mode in MODES)
        work_gain = 100 * (fixed["work"] - adaptive["work"]) / fixed["work"]
        interval_gain = 100 * (fixed["p95"] - adaptive["p95"]) / fixed["p95"]
        lines.extend([
            "",
            f"**{scene}:** submitted-frame work average relative reduction "
            f"{work_gain:+.2f}%; submission interval p95 relative reduction "
            f"{interval_gain:+.2f}%. Negative means regression.",
            "",
        ])
    lines += [
        "Do not equate a stable, refresh-paced frame rate with absence of CPU",
        "benefit; CPU savings may increase idle headroom without changing",
        "submission cadence. The Xvfb run cannot validate the user's physical",
        "display smoothness; a local Windows/macOS same-machine A/B is required",
        "for that claim. All timing comparisons are diagnostic, not CI budgets.",
        "",
    ]
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=35)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)

    rows = []
    for trial in TRIALS:
        order = MODES if trial % 2 else tuple(reversed(MODES))
        for mode in order:
            for scene, size, cover in SCENES:
                rows.append(run_one(binary, out, trial, mode, scene, size, cover, args.timeout))
    summary = summarize(out, rows)
    (out / "summary.md").write_text(summary, encoding="utf-8")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    print(summary)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"AKULA_WINDOW_SCROLL_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
