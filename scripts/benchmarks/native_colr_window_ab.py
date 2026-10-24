#!/usr/bin/env python3
"""Pinned before/after Akula COLRv1 emoji window-scroll A/B, Linux/Xvfb.

This is real Winit/Masonry/softbuffer work and per-process /proc RSS.
It is NOT a physical-display scanout test, and fontconfig matching is
not proof of which font Parley selected for every grapheme.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from statistics import median

from native_scroll_window_ab import fixture, number

SCENES = (("colr_compact", "1000x600"), ("colr_large", "1750x1051"))
TRIALS = (1, 2, 3)
REVISION = ("before", "after")


def peak_rss_kib(pid: int) -> int:
    try:
        data = Path(f"/proc/{pid}/status").read_text()
    except (FileNotFoundError, ProcessLookupError):
        return 0
    for label in ("VmHWM:", "VmRSS:"):
        for line in data.splitlines():
            if line.startswith(label):
                return int(line.split()[1])
    return 0


def one(binary: Path, out: Path, trial: int, revision: str,
        scene: str, size: str, timeout: float) -> dict:
    root = out / scene / f"trial-{trial}" / revision
    vault, data_dir = root / "vault", root / "data"
    vault.mkdir(parents=True, exist_ok=True)
    data_dir.mkdir(parents=True, exist_ok=True)
    note = vault / "colr-scroll.md"
    source = fixture(False, emoji_colrv1=True)
    note.write_text(source, encoding="utf-8")
    digest = hashlib.sha256(source.encode("utf-8")).hexdigest()
    replay_file, metric_file, logfile = (
        root / "replay.json", root / "metrics.json", root / "process.log"
    )
    cmd = [
        str(binary), "--vault", str(vault), "--data-dir", str(data_dir),
        "--open", str(note), "--threads", "2", "--size", size,
        "--replay-scroll", str(replay_file), "--metrics", str(metric_file),
    ]
    env = os.environ.copy()
    env["AKULA_VELLO_SINGLE_RECT"] = "1"
    env.pop("AKULA_EXPERIMENTAL_VELLO_SINGLE_RECT", None)
    peak = 0
    with logfile.open("wb") as log:
        proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, env=env)
        start = time.monotonic()
        try:
            while proc.poll() is None:
                peak = max(peak, peak_rss_kib(proc.pid))
                if time.monotonic() - start > timeout:
                    raise TimeoutError(f"{scene}/{trial}/{revision}: native replay timeout")
                time.sleep(0.05)
            if proc.returncode != 0:
                raise RuntimeError(
                    f"{scene}/{trial}/{revision}: exit {proc.returncode}; {logfile}"
                )
        except BaseException:
            proc.kill()
            proc.wait()
            raise
    if peak == 0:
        raise RuntimeError(f"{scene}/{trial}/{revision}: missing Linux process RSS")
    try:
        replay = json.loads(replay_file.read_text())
        metrics = json.loads(metric_file.read_text())
    except (OSError, ValueError) as error:
        raise RuntimeError(f"{scene}/{trial}/{revision}: missing valid GUI output") from error
    frames = number(replay, "frames", 20)
    if len(replay.get("quarters", [])) != 4:
        raise ValueError(f"{scene}: the four scroll replay quarters are missing")
    for key in ("fps", "workAvgMs", "intervalP95Ms"):
        number(replay, key, 0.0001)
    for key in ("frameP95Ms", "rasterP95Ms"):
        number(metrics, key, 0.0001)
    if number(metrics, "threads") != 2 or number(metrics, "frames") < frames:
        raise ValueError(f"{scene}: wrong worker count or inconsistent frame count")
    row = {
        "scene": scene, "size": size, "trial": trial, "revision": revision,
        "source_sha256": digest, "peak_rss_kib": peak,
        "replay": replay, "metrics": metrics,
    }
    (root / "sample.json").write_text(json.dumps(row, indent=2) + "\n")
    print(
        f"AKULA_COLR_GUI trial={trial} scene={scene} revision={revision} "
        f"frames={frames:.0f} work_avg_ms={replay['workAvgMs']:.2f} "
        f"raster_p95_ms={metrics['rasterP95Ms']:.2f} "
        f"frame_p95_ms={metrics['frameP95Ms']:.2f} peak_rss_kib={peak}",
        flush=True,
    )
    return row


def report(rows: list[dict], before_commit: str, after_commit: str,
           font_identity: str) -> str:
    expected = len(SCENES) * len(TRIALS) * len(REVISION)
    if len(rows) != expected:
        raise ValueError(f"incomplete matrix {len(rows)} of {expected}")
    lines = [
        "# COLRv1 emoji-rich native-window replay: pinned A/B",
        "",
        f"Baseline: \`{before_commit}\`; candidate: \`{after_commit}\`.",
        f"Fontconfig probe: \`{font_identity}\`.",
        "Identical UTF-8 Markdown and 2-worker adaptive Vello, Linux",
        "Xvfb/Openbox, release native Winit/Masonry/softbuffer binaries.",
        "Fresh process and vault for every trial, alternating before/after.",
        "Note: fontconfig probe is **not** per-glyph Parley proof.",
        "Physical display FPS, scanout and latency are **not** measured.",
        "RSS is /proc VmHWM sampled while each process is alive;",
        "sampling may miss short-lived startup peaks.",
        "",
        "| Scene | Revision | Work avg (ms) | Raster p95 (ms) | "
        "Frame p95 (ms) | Peak RSS (MiB) |",
        "|---|---|---:|---:|---:|---:|",
    ]
    for scene, _ in SCENES:
        matching = [r for r in rows if r["scene"] == scene]
        if len({r["source_sha256"] for r in matching}) != 1:
            raise ValueError(f"{scene}: different fixture data")
        stats = {}
        for revision in REVISION:
            r = [x for x in matching if x["revision"] == revision]
            if sorted(x["trial"] for x in r) != list(TRIALS):
                raise ValueError(f"{scene}: incomplete trial set for {revision}")
            work = median(float(x["replay"]["workAvgMs"]) for x in r)
            raster = median(float(x["metrics"]["rasterP95Ms"]) for x in r)
            frame = median(float(x["metrics"]["frameP95Ms"]) for x in r)
            rss = median(x["peak_rss_kib"] for x in r) / 1024
            stats[revision] = (work, raster, frame, rss)
            lines.append(
                f"| {scene} | {revision} | {work:.3f} | {raster:.3f} | "
                f"{frame:.3f} | {rss:.1f} |"
            )
        old, new = stats["before"], stats["after"]
        delta_work = 100 * (new[0] - old[0]) / old[0]
        delta_rss = 100 * (new[3] - old[3]) / old[3]
        lines.extend([
            "",
            f"**{scene}:** work difference candidate vs baseline "
            f"{delta_work:+.2f}%, sampled peak RSS {delta_rss:+.2f}%.",
            "No regression threshold: diagnostic only; same-runner observations",
            "cannot alone establish that the page selected a COLRv1 glyph.",
            "",
        ])
    return "\n".join(lines)


def main() -> int:
    args_parser = argparse.ArgumentParser()
    args_parser.add_argument("--before", type=Path, required=True)
    args_parser.add_argument("--after", type=Path, required=True)
    args_parser.add_argument("--out", type=Path, required=True)
    args_parser.add_argument("--before-commit", required=True)
    args_parser.add_argument("--after-commit", required=True)
    args_parser.add_argument("--timeout", type=float, default=40)
    args = args_parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    binary = {
        "before": args.before.resolve(strict=True),
        "after": args.after.resolve(strict=True),
    }
    # A real COLRv1 font installed into fontconfig is a prerequisite;
    # otherwise Ubuntu's default bitmap emoji fonts produce a false negative.
    font_identity = subprocess.check_output(
        ["fc-match", "-f", "%{family}\n", "Noto Color Emoji:charset=1f601"],
        text=True,
    ).strip().splitlines()[0]
    if "twemoji_smiley-glyf_colr_1" not in font_identity:
        raise RuntimeError(f"COLRv1 emoji font was not selected: {font_identity}")
    rows = []
    for trial in TRIALS:
        order = REVISION if trial % 2 else tuple(reversed(REVISION))
        for scene, size in SCENES:
            for revision in order:
                rows.append(one(binary[revision], out, trial, revision, scene, size, args.timeout))
    summary = report(rows, args.before_commit, args.after_commit, font_identity)
    (out / "summary.md").write_text(summary + "\n")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(summary)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"AKULA_COLR_GUI_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
