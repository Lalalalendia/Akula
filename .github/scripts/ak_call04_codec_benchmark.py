#!/usr/bin/env python3
"""Offline AK-CALL-04 reproducible screen trace + software codec comparison.

Runs an actual Rust screen_damage_lab planner (when available), x264 and AV1
on the SAME deterministic BGRA trace. No capture, traffic, network or UI.
This is diagnostic evidence, NOT a production codec selection gate.
"""
import argparse
import json
import math
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

try:
    import resource
except ImportError:
    resource = None

REPO = Path(__file__).resolve().parents[2]


def exec_timed(cmd):
    before = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    start = time.perf_counter()
    done = subprocess.run(cmd, capture_output=True, text=True, check=False)
    wall = time.perf_counter() - start
    if done.returncode:
        raise RuntimeError(f"command failed ({done.returncode}): {cmd}\n{done.stderr[-4000:]}")
    after = resource.getrusage(resource.RUSAGE_CHILDREN) if resource else None
    cpu = ((after.ru_utime + after.ru_stime) -
           (before.ru_utime + before.ru_stime)) if before else None
    return done.stdout, round(wall, 4), round(cpu, 4) if cpu is not None else None


def screen_row(width, world_y):
    result = bytearray(width * 4)
    for x in range(width):
        p = (x * 7 + world_y * 11 + (x // 48) * 71) & 255
        offset = x * 4
        result[offset:offset + 4] = bytes((p, (p * 3 + world_y) & 255,
                                            (p * 5 + x // 7) & 255, 255))
    return result


def paint_edit(frame, width, height, step):
    box_w, box_h = min(80, width), min(44, height)
    x = (step * 37) % (width - box_w + 1)
    y = (step * 19) % (height - box_h + 1)
    pixel = bytes(((30 + step * 17) & 255, 200, 248, 255))
    for row in range(y, y + box_h):
        offset = (row * width + x) * 4
        frame[offset:offset + box_w * 4] = pixel * box_w


def generate_trace(path, scenario, width, height, frames):
    initial = b"".join(screen_row(width, y) for y in range(height))
    previous = initial
    with path.open("wb") as out:
        for index in range(frames):
            if scenario == "edits":
                pixels = bytearray(initial)
                if index // 4 % 5:
                    paint_edit(pixels, width, height, index // 4)
                previous = bytes(pixels)
            elif scenario == "scroll":
                if index:
                    previous = (previous[8 * width * 4:] +
                                b"".join(screen_row(width, height + (index - 1) * 8 + y)
                                         for y in range(8)))
            elif scenario == "motion":
                pixels = bytearray(initial)
                for p in range(0, len(pixels), 4):
                    pixels[p] = (pixels[p] + index * 17) & 255
                    pixels[p + 1] = (pixels[p + 1] + index * 13) & 255
                    pixels[p + 2] = (pixels[p + 2] + index * 7) & 255
                previous = bytes(pixels)
            else:
                raise ValueError(scenario)
            out.write(previous)


def rgb_psnr(original, decoded):
    if original.stat().st_size != decoded.stat().st_size:
        raise RuntimeError("decoded BGRA frame count differs from input")
    squared = 0
    channels = 0
    with original.open("rb") as src, decoded.open("rb") as dst:
        while True:
            a = src.read(256 * 1024)
            b = dst.read(256 * 1024)
            if not a:
                break
            if len(a) != len(b) or len(a) % 4:
                raise RuntimeError("invalid decoded BGRA alignment")
            for p in range(0, len(a), 4):
                for k in range(3):
                    diff = a[p + k] - b[p + k]
                    squared += diff * diff
                channels += 3
    if not channels:
        raise RuntimeError("empty trace")
    return (None, True) if squared == 0 else (round(10 * math.log10(255 * 255 * channels / squared), 3), False)


def measure_codec(trace, tmp, scenario, width, height, fps, codec):
    encoded = tmp / f"{scenario}-{codec}.mkv"
    decoded = tmp / f"{scenario}-{codec}.bgra"
    cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
           "-f", "rawvideo", "-pixel_format", "bgra", "-video_size",
           f"{width}x{height}", "-framerate", str(fps), "-i", str(trace),
           "-an", "-threads", "2"]
    if codec == "h264":
        cmd += ["-c:v", "libx264", "-preset", "veryfast", "-tune", "zerolatency",
                "-g", str(fps * 2), "-bf", "0", "-crf", "18"]
    else:
        cmd += ["-c:v", "libaom-av1", "-cpu-used", "8", "-row-mt", "1",
                "-lag-in-frames", "0", "-g", str(fps * 2), "-b:v", "0", "-crf", "30"]
    cmd += ["-pix_fmt", "yuv444p", str(encoded)]
    _, encode_wall_s, encode_cpu_s = exec_timed(cmd)
    exec_timed(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y",
                "-i", str(encoded), "-vsync", "0", "-pix_fmt", "bgra",
                "-f", "rawvideo", str(decoded)])
    psnr, exact = rgb_psnr(trace, decoded)
    size = encoded.stat().st_size
    return {"container_bytes": size,
            "encode_wall_s": encode_wall_s, "encode_cpu_s": encode_cpu_s,
            "rgb_psnr_db": psnr, "rgb_exact": exact}


def get_lab_executable(skip_lab):
    if skip_lab:
        return None
    if not shutil.which("cargo"):
        raise RuntimeError("cargo is required for actual Rust lab metrics; use --codec-only for a codec smoke test")
    manifest = REPO / "aquilum-app" / "core" / "Cargo.toml"
    exec_timed(["cargo", "build", "--locked", "--manifest-path", str(manifest),
                "--example", "ak_call04_damage_metrics", "--features", "screen-damage-lab"])
    suffix = ".exe" if __import__("os").name == "nt" else ""
    path = REPO / "aquilum-app" / "target" / "debug" / "examples" / ("ak_call04_damage_metrics" + suffix)
    if not path.is_file():
        raise RuntimeError(f"built Rust example missing: {path}")
    return path


def run_lab(exe, scenario, width, height, frames, trace):
    output, wall, cpu = exec_timed([str(exe), scenario, str(width), str(height),
                                    str(frames), str(trace)])
    result = {}
    for token in output.strip().split():
        key, eq, value = token.partition("=")
        if eq:
            result[key] = int(value) if value.isdecimal() else value
    if result.get("frames") != frames or result.get("scenario") != scenario:
        raise RuntimeError(f"unexpected Rust lab output: {output}")
    result["measure_wall_s"] = wall
    result["measure_cpu_s"] = cpu
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", choices=["all", "edits", "scroll", "motion"], default="all")
    parser.add_argument("--width", type=int, default=640)
    parser.add_argument("--height", type=int, default=360)
    parser.add_argument("--frames", type=int, default=24)
    parser.add_argument("--fps", type=int, default=12)
    parser.add_argument("--codec-only", action="store_true", help="skip actual Rust delta metric (not a complete comparison)")
    args = parser.parse_args()
    if not (64 <= args.width <= 1920 and 64 <= args.height <= 1080 and
            2 <= args.frames <= 120 and 1 <= args.fps <= 60 and
            args.width * args.height * 4 * args.frames <= 256 * 1024 * 1024):
        parser.error("limits: 64..1920 width, 64..1080 height, 2..120 frames, 1..60 fps, <=256 MiB raw trace")
    if not shutil.which("ffmpeg"):
        parser.error("ffmpeg with libx264 and libaom-av1 is required")
    binary = get_lab_executable(args.codec_only)
    scenarios = ["edits", "scroll", "motion"] if args.scenario == "all" else [args.scenario]
    ffmpeg_version = subprocess.run(["ffmpeg", "-version"], capture_output=True, text=True, check=True).stdout.splitlines()[0]
    with tempfile.TemporaryDirectory(prefix="ak-call04-bench-") as temp:
        tmp = Path(temp)
        for scenario in scenarios:
            trace = tmp / f"{scenario}.bgra"
            generate_trace(trace, scenario, args.width, args.height, args.frames)
            report = {"scenario": scenario, "width": args.width, "height": args.height,
                      "frames": args.frames, "fps": args.fps, "ffmpeg": ffmpeg_version,
                      "bgra_source_bytes": trace.stat().st_size,
                      "rust_lab": run_lab(binary, scenario, args.width, args.height, args.frames, trace) if binary else None,
                      "h264": measure_codec(trace, tmp, scenario, args.width, args.height, args.fps, "h264"),
                      "av1": measure_codec(trace, tmp, scenario, args.width, args.height, args.fps, "av1")}
            print(json.dumps(report, sort_keys=True))
    print("NOTE: Rust counts raw BGRA tile/move bytes and excludes protocol overhead; codecs are software, lossy, and use DIFFERENT CRFs. Compare PSNR, CPU, GPU and actual packet bytes before any go/no-go decision.")


if __name__ == "__main__":
    main()
