#!/usr/bin/env python3
"""Run the real Akula window and measure first visible reader frame + process RAM.

The provided book stays local. This script never uploads EPUB bytes, excerpts,
book metadata, or screenshots to the network. CI uses synthetic EPUBs only.
"""
from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

FRAME = re.compile(r"READER_GUI_FRAME\s+([^\r\n]+)")


def args_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--book", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=65)
    parser.add_argument("--screenshot", action="store_true")
    return parser


def run(binary: Path, book: Path, out: Path, timeout: float, screenshot: bool) -> dict:
    binary = binary.resolve(strict=True)
    book = book.resolve(strict=True)
    if not book.is_file() or book.suffix.lower() != ".epub":
        raise ValueError("Expected a local .epub file")
    out.mkdir(parents=True, exist_ok=True)
    vault, data = out / "vault", out / "data"
    vault.mkdir(exist_ok=True)
    data.mkdir(exist_ok=True)
    (vault / "sample.md").write_text("# GUI Reader test\n", encoding="utf-8")
    script = out / "script.txt"
    # Pauses are in milliseconds. The Reader opens after Akula's first frame.
    # Keep the window alive for screenshots after the first glyph frame.
    script.write_text(
        f"0 reader {book}\n0 mark reader-opened\n5000 wait\n"
        "0 close-reader\n0 mark reader-closed\n", encoding="utf-8"
    )
    log = out / "app.log"
    metrics = out / "startup.json"
    probe = out / "memory-probe.md"
    command = [str(binary), "--vault", str(vault), "--data-dir", str(data),
               "--size", "1200x800", "--threads", "4", "--memory-probe",
               str(probe), "--metrics", str(metrics)]
    import os
    env = dict(os.environ)
    env["AQ_PROBE_SCRIPT"] = str(script)
    started = time.monotonic()
    captured = False
    screenshot_path = out / "reader.png"
    with log.open("wb") as stream:
        process = subprocess.Popen(command, stdout=stream, stderr=subprocess.STDOUT, env=env)
        try:
            while process.poll() is None and time.monotonic() - started < timeout:
                output = log.read_text(encoding="utf-8", errors="replace")
                if not captured and FRAME.search(output):
                    if screenshot:
                        screenshot_tool = shutil.which("import")
                        if not screenshot_tool:
                            raise RuntimeError("ImageMagick 'import' not found")
                        subprocess.run([screenshot_tool, "-window", "root", str(screenshot_path)],
                                       check=True, timeout=12)
                    captured = True
                time.sleep(0.15)
            if process.poll() is None:
                raise TimeoutError(f"Akula did not exit in {timeout:.0f} seconds")
            if process.returncode != 0:
                raise RuntimeError(f"Akula exited with code {process.returncode}")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)

    output = log.read_text(encoding="utf-8", errors="replace")
    matches = FRAME.findall(output)
    if len(matches) != 1:
        raise AssertionError(f"Expected exactly one first-text-present line, got {len(matches)}; see {log}")
    values = {}
    for item in matches[0].split():
        if "=" in item:
            k, v = item.split("=", 1)
            values[k] = v
    required = ["first_text_present_ms", "sync_open_ms", "heap_before_mib",
                "heap_at_present_mib", "heap_peak_mib", "rss_before_kib",
                "rss_at_present_kib", "process_hwm_kib"]
    for k in required:
        if k not in values:
            raise AssertionError(f"Missing metric: {k}")
    for key in required:
        values[key] = float(values[key])
    if not 0 <= values["sync_open_ms"] <= values["first_text_present_ms"]:
        raise AssertionError("Invalid presentation timeline")
    if not values["heap_peak_mib"] >= values["heap_at_present_mib"]:
        raise AssertionError("Allocation peak lower than live heap")
    if sys.platform.startswith("linux"):
        if not (values["rss_before_kib"] > 0 and values["rss_at_present_kib"] > 0
                and values["process_hwm_kib"] >= values["rss_at_present_kib"]):
            raise AssertionError("Linux memory counters not available")
    if screenshot and not screenshot_path.is_file():
        raise AssertionError("Reader screenshot missing")
    values["sample_type"] = "Akula window/softbuffer first text-containing submitted frame"
    values["first_pixel_scanout_measured"] = False
    values["book_bytes"] = book.stat().st_size
    values["elapsed_process_s"] = round(time.monotonic() - started, 3)
    (out / "result.json").write_text(json.dumps(values, indent=2) + "\n", encoding="utf-8")
    return values


def main() -> int:
    opts = args_parser().parse_args()
    try:
        result = run(opts.binary, opts.book, opts.out, opts.timeout, opts.screenshot)
    except Exception as exc:
        print(f"READER_GUI_PROBE_FAILURE {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    print("READER_GUI_PROBE_RESULT " + json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
