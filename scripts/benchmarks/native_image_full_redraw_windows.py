#!/usr/bin/env python3
"""IMG-PERF-08: real-Win32 mixed-image full-redraw LOD policy A/B.

No system Python extensions required. Every PNG has distinct content and hash.
Developer-only full redraw forces new image raster instead of scroll-row copies.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path
from statistics import median
from math import floor, isfinite

from native_mipmap_window_ab import windows_gui_witness, rss_kib
from native_mipmap_windows_ab import windows_scenes

COUNTS = (1, 10, 100)
MODES = ("eager", "selective_l2")
TRIALS = (1, 2)
WIDTH = 1280
HEIGHT = 884
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def chunk(kind: bytes, payload: bytes) -> bytes:
    crc = zlib.crc32(kind + payload) & 0xFFFF_FFFF
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", crc)


def make_image(image_id: int) -> bytes:
    """Compression-friendly deterministic RGBA PNG, unique at every ID."""
    compressed = bytearray()
    compressor = zlib.compressobj(level=6)
    for band in range((HEIGHT + 15) // 16):
        row = bytearray()
        for col in range(WIDTH // 32):
            red = (image_id * 13 + band * 9 + col * 23) % 256
            green = (image_id * 29 + band * 19 + col * 7) % 256
            blue = (image_id * 47 + band * 3 + col * 17) % 256
            alpha = 192 if (image_id + band + col) % 11 == 0 else 255
            row.extend(bytes((red, green, blue, alpha)) * 32)
        raw = bytes(row)
        for _ in range(min(16, HEIGHT - band * 16)):
            compressed.extend(compressor.compress(b"\x00" + raw))
    compressed.extend(compressor.flush())
    return (
        PNG_MAGIC
        + chunk(b"IHDR", struct.pack(">IIBBBBB", WIDTH, HEIGHT, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", bytes(compressed))
        + chunk(b"IEND", b"")
    )


def assets(out: Path, n: int) -> tuple[Path, dict[str, str]]:
    directory = out / "fixtures" / str(n)
    directory.mkdir(parents=True, exist_ok=True)
    result = {}
    for index in range(n):
        filename = f"photo_{index:03d}.png"
        target = directory / filename
        if not target.exists():
            target.write_bytes(make_image(index + 1))
        data = target.read_bytes()
        if not data.startswith(PNG_MAGIC):
            raise ValueError("PNG creation failed")
        result[filename] = hashlib.sha256(data).hexdigest()
    if len(set(result.values())) != n:
        raise ValueError("PNG fixtures must have distinct bytes and hashes")
    return directory, result


def document(n: int) -> str:
    # Mirror the *known working* 240-section note in the original
    # city-cover Windows A/B. A one-image/one-heading document does
    # not scroll; it previously yielded zero actual mip requests.
    # Always keep real scrollable text and repeat early images, while
    # the 10/100-image runs also visit every distinct source filename.
    parts = [
        "# IMG-PERF-08 multi-image native replay\n\n",
        "![200](photo_000.png)\n\n",
        "![200|crop=10,10,90,90](photo_000.png)\n\n",
    ]
    for i in range(240):
        parts.append(
            f"## Section {i}\n"
            "Markdown, **bold**, *italic*, unicode, [[Akula]] and wrapped text.\n\n"
        )
        if i < n or i % 12 == 0:
            chosen = i if i < n else i % n
            parts.append(f"![200](photo_{chosen:03d}.png)\n\n")
        if i % 24 == 0:
            parts.append(f"![200|crop=10,10,90,90](photo_{(i % n):03d}.png)\n\n")
    return "".join(parts)


def scan_anchors(note: str, n: int) -> list[int]:
    """Byte offsets of distinct PNG paragraphs, not guessed scroll distances.

    Visit up to 32 unique images twice in 6 seconds (forward then reverse).
    Do not use all 100 at an 8ms wheel frequency: asynchronous decoding and
    viewport composition would never display most sources.
    """
    offsets = []
    for image_id in range(min(n, 32)):
        marker = f"![200](photo_{image_id:03d}.png)"
        char_offset = note.find(marker)
        if char_offset < 0:
            raise ValueError(f"fixture missing displayed image {image_id}")
        offsets.append(len(note[:char_offset].encode("utf-8")))
    if not offsets or len(set(offsets)) != len(offsets) or offsets != sorted(offsets):
        raise ValueError("image scan offsets must be unique and strictly increasing")
    return offsets


def percentile(samples: list[float], fraction: float) -> float:
    if len(samples) < 80 or not all(isinstance(x, (int, float)) and not isinstance(x, bool)
                                   and isfinite(x) and x >= 0 for x in samples):
        raise ValueError("Invalid or insufficient replay CPU frame samples")
    ordered = sorted(samples)
    return ordered[floor((len(samples) - 1) * fraction + 0.5)]


def run_one(binary: Path, root: Path, sample: tuple[int, int, str],
            source_dir: Path, hashes: dict[str, str], viewport: str,
            timeout: float) -> dict:
    images, trial, mode = sample
    folder = root / "runs" / f"images-{images}" / f"trial-{trial}" / mode
    vault = folder / "vault"
    data = folder / "data"
    vault.mkdir(parents=True, exist_ok=True)
    data.mkdir(parents=True, exist_ok=True)
    for name in hashes:
        shutil.copyfile(source_dir / name, vault / name)
    note = document(images)
    anchors = scan_anchors(note, images)
    (vault / "images.md").write_text(note, encoding="utf-8")
    metrics = folder / "metrics.json"
    replay = folder / "replay.json"
    log_path = folder / "process.log"
    args = [
        str(binary), "--vault", str(vault), "--data-dir", str(data),
        "--open", str(vault / "images.md"), "--threads", "2",
        "--size", viewport, "--replay-fling", str(replay),
        "--metrics", str(metrics),
    ]
    env = os.environ.copy()
    env["AKULA_EXPERIMENTAL_IMAGE_MIP"] = "1"
    env["AKULA_EXPERIMENTAL_IMAGE_MIP_FILTER"] = "medium"
    env["AKULA_EXPERIMENTAL_IMAGE_MIP_LINGER"] = "1"
    env["AKULA_EXPERIMENTAL_IMAGE_MIP_PROOF"] = "1"
    env["AKULA_EXPERIMENTAL_IMAGE_FORCE_FULL_REDRAW"] = "1"
    env["AKULA_EXPERIMENTAL_IMAGE_SCAN_BYTES"] = ",".join(map(str, anchors))
    env.pop("AKULA_EXPERIMENTAL_IMAGE_SCROLL_SCAN", None)
    env["AKULA_EXPERIMENTAL_IMAGE_DISTINCT_PROOF"] = "1"
    env["AKULA_EXPERIMENTAL_FRAME_SAMPLES"] = "1"
    env["AKULA_EXPERIMENTAL_FRAME_STAGES"] = "1"
    if mode == "selective_l2":
        env["AKULA_EXPERIMENTAL_IMAGE_MIP_DIRECT_L2"] = "1"
    else:
        env.pop("AKULA_EXPERIMENTAL_IMAGE_MIP_DIRECT_L2", None)
    # The same sampler is used in both strategies; this is a policy+reducer
    # comparison, not a Medium-vs-High A/B.
    env.pop("AKULA_EXPERIMENTAL_IMAGE_FILTER_FORCE", None)
    peak_kib = 0
    witness = None
    with log_path.open("wb") as log:
        process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, env=env)
        begin = time.monotonic()
        try:
            while process.poll() is None:
                peak_kib = max(peak_kib, rss_kib(process.pid))
                if witness is None:
                    witness = windows_gui_witness(process.pid)
                if time.monotonic() - begin > timeout:
                    raise TimeoutError(f"{sample}: run timed out")
                time.sleep(0.05)
            if process.returncode:
                raise RuntimeError(f"{sample}: exit {process.returncode}, see {log_path}")
        except BaseException:
            process.kill()
            process.wait()
            raise

    log = log_path.read_text(encoding="utf-8", errors="replace")
    if witness is None or witness.get("title_akula") is not True or not witness.get("uncloaked"):
        raise ValueError(f"{sample}: missing PID-owned uncloaked Win32 window")
    if peak_kib <= 0:
        raise ValueError(f"{sample}: no process peak working set")
    if "AKULA_MIP_APPLIED level=" not in log:
        raise ValueError(f"{sample}: image mipmap was never used")
    if "AKULA_IMAGE_REPLAY_FULL_REDRAW_APPLIED" not in log:
        raise ValueError(f"{sample}: full redraw was not enabled")
    scan_witness = f"AKULA_IMAGE_ANCHOR_SCAN_APPLIED count={len(anchors)} schedule=forward-reverse"
    if scan_witness not in log:
        raise ValueError(f"{sample}: native byte-anchor image scan did not occur")
    if "AKULA_IMAGE_ANCHOR_SCAN_SEEK index=" not in log:
        raise ValueError(f"{sample}: no actual editor image scroll anchor processed")
    parsed_replay = json.loads(replay.read_text(encoding="utf-8"))
    parsed_metrics = json.loads(metrics.read_text(encoding="utf-8"))
    work = parsed_replay.get("workSamplesMs")
    stage = parsed_replay.get("stageSamples")
    whole_stage = parsed_metrics.get("frameStages")
    if not isinstance(work, list) or not isinstance(stage, list) or len(stage) != len(work):
        raise ValueError(f"{sample}: missing aligned replay CPU/stage samples")
    if not isinstance(whole_stage, list) or len(whole_stage) != parsed_metrics.get("frames"):
        raise ValueError(f"{sample}: missing whole-process stages")
    # A valid six-second image scan must keep painting between anchor
    # transitions. In the previous run there were only two CPU replay
    # frames; that invalidated the cache frame-count diagnostic entirely.
    if len(work) < 80:
        raise ValueError(
            f"{sample}: only {len(work)} rendered replay frames in six seconds; "
            "developer image dwell must request full redraw every replay tick"
        )
    lines = [line for line in log.splitlines() if line.startswith("AKULA_MIP_CACHE ")]
    if not lines:
        raise ValueError(f"{sample}: no source/eviction diagnostic line")
    counters = {
        item.split("=", 1)[0]: item.split("=", 1)[1]
        for item in lines[-1].split()[1:] if "=" in item
    }
    unique = int(counters.get("distinct_requested_ids", 0))
    unique_pixels = int(counters.get("distinct_sampled_pixel_fingerprints", 0))
    if unique_pixels > unique:
        raise ValueError(f"{sample}: pixel fingerprint count exceeds distinct IDs")
    # At 1280x884, 24 eager L1+L2 pyramids exceed the real 32-MiB
    # budget. A 100-file note with only a handful of visible images
    # would not constitute cache-pressure evidence.
    minimum = {1: 1, 10: 8, 100: 24}[images]
    if unique < minimum or unique_pixels < minimum:
        raise ValueError(
            f"{sample}: only {unique} runtime IDs and {unique_pixels} "
            f"independently sampled decoded-pixel fingerprints for "
            f"{images} seeded PNG files; both must reach at least {minimum}"
        )
    p95, p99 = percentile(work, .95), percentile(work, .99)
    raster = [float(x["rasterPixels"]) for x in stage]
    if min(raster, default=0) <= 0 or median(raster) < 0.45 * int(viewport.split("x")[0]) * int(viewport.split("x")[1]):
        raise ValueError(f"{sample}: damaged pixel area is too small for full-redraw experiment")
    generation = sum(int(x["mipGenerated"]) for x in whole_stage)
    if generation < 1:
        raise ValueError(f"{sample}: no mip generated")
    summary = {
        "count": images, "trial": trial, "mode": mode, "viewport": viewport,
        "binary_image_sha256": hashlib.sha256(
            "".join(f"{k}:{hashes[k]};" for k in sorted(hashes)).encode()
        ).hexdigest(),
        "note_sha256": hashlib.sha256(note.encode()).hexdigest(),
        "unique_image_sources_in_note": len(hashes),
        "replay_image_anchor_count": len(anchors),
        "replay_anchor_dwell_ms_each_pass": 3000 / len(anchors),
        "unique_mip_source_ids_requested": unique,
        "distinct_decoded_pixel_fingerprints": unique_pixels,
        "cpu_p95_ms": p95, "cpu_p99_ms": p99,
        "cpu_mean_ms": sum(work) / len(work),
        "full_damage_pixels_median": median(raster),
        "mip_build_all_frames_ms": sum(float(x["mipBuildMs"]) for x in whole_stage),
        "mip_build_replay_ms": sum(float(x["mipBuildMs"]) for x in stage),
        "mip_generated_levels": generation,
        "cache_final_hits": int(counters["final_hits"]),
        "cache_evicted": int(counters["evicted"]),
        "cache_resident_levels": int(counters["resident_levels"]),
        "peak_rss_mib": peak_kib / 1024,
        "native_window": witness,
    }
    (folder / "sample.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print("AKULA_IMAGE_FULL_REDRAW " + json.dumps(summary, sort_keys=True), flush=True)
    return summary


def report(rows: list[dict], binary_hash: str) -> str:
    if len(rows) != len(COUNTS) * len(MODES) * len(TRIALS):
        raise ValueError("Incomplete full-redraw 1/10/100 image A/B matrix")
    lines = [
        "# IMG-PERF-08: distinct PNGs, native Win32 forced-full-redraw",
        "", f"Binary SHA256: {binary_hash}.",
        "All processes used the same Medium+mip sampler, viewport and cache budget.",
        "Eager uses scalar L1→L2; selective uses direct packed L2.",
        "No claim of pure policy-only timing: reducer and retention both change.",
        "All sources have distinct PNG SHA256; actual touched source IDs are reported.",
        "Replay uses 6s timed forward-reverse real-editor image byte-anchor seeks",
        "plus developer-only full raster of every drawn frame.",
        "Work times are CPU, not physical scanout/FPS. Image-quality comparison is separate.",
        "",
        "| Sources in note | Mode | Runtime IDs | Distinct pixel fingerprints | CPU p95 ms | CPU p99 ms | Replay mip build ms | Peak RSS MiB |",
        "|---:|---|---:|---:|---:|---:|---:|---:|",
    ]
    for count in COUNTS:
        parts = [row for row in rows if row["count"] == count]
        for field in ("note_sha256", "binary_image_sha256"):
            if len({row[field] for row in parts}) != 1:
                raise ValueError(f"{count}: inputs differ between modes")
        mode_aggregates = {}
        for mode in MODES:
            group = [row for row in parts if row["mode"] == mode]
            if sorted(x["trial"] for x in group) != list(TRIALS):
                raise ValueError(f"{count}/{mode}: paired trial missing")
            keys = ("unique_mip_source_ids_requested", "distinct_decoded_pixel_fingerprints",
                    "cpu_p95_ms", "cpu_p99_ms",
                    "mip_build_replay_ms", "peak_rss_mib")
            averages = {key: median(float(row[key]) for row in group) for key in keys}
            mode_aggregates[mode] = averages
            lines.append(
                f"| {count} | {mode} | {averages['unique_mip_source_ids_requested']:.0f} | "
                f"{averages['distinct_decoded_pixel_fingerprints']:.0f} | "
                f"{averages['cpu_p95_ms']:.3f} | {averages['cpu_p99_ms']:.3f} | "
                f"{averages['mip_build_replay_ms']:.3f} | {averages['peak_rss_mib']:.1f} |"
            )
        deltas = []
        for trial in TRIALS:
            base = next(row for row in parts if row["mode"] == "eager" and row["trial"] == trial)
            candidate = next(row for row in parts if row["mode"] == "selective_l2" and row["trial"] == trial)
            deltas.append(candidate["cpu_p95_ms"] - base["cpu_p95_ms"])
        lines.append(f"{count} sources: paired CPU p95 delta (direct-eager) ms: {[round(x,3) for x in deltas]}.")
        lines.append("")
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=90)
    args = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("This stress experiment requires a Windows runner")
    binary = args.binary.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    viewport = windows_scenes()[1][1]
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    rows = []
    for count in COUNTS:
        src, hashes = assets(out, count)
        for trial in TRIALS:
            order = MODES if trial % 2 else tuple(reversed(MODES))
            for mode in order:
                rows.append(run_one(binary, out, (count, trial, mode), src, hashes, viewport, args.timeout))
    summary = report(rows, binary_hash)
    (out / "summary.md").write_text(summary, encoding="utf-8")
    (out / "matrix.json").write_text(json.dumps(rows, indent=2), encoding="utf-8")
    (out / "fixture_hashes.json").write_text(json.dumps(
        {str(count): assets(out, count)[1] for count in COUNTS}, indent=2
    ), encoding="utf-8")
    print(summary, flush=True)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        print(f"AKULA_IMAGE_FULL_REDRAW_ERROR {type(exc).__name__}: {exc}", file=sys.stderr)
        raise SystemExit(1)
