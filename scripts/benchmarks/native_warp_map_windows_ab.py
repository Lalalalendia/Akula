#!/usr/bin/env python3
"""Strict WARP staging map mode comparison. Native Windows GUI, developer build only."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from statistics import median
import subprocess
import sys
import time

from native_mipmap_window_ab import windows_gui_witness, windows_peak_rss_kib

MODES = ("write", "read_write")
TRIALS = tuple(range(1, 7))  # six alternating pairs, twelve independent GUI processes
TEXT = "# WARP staging scroll\n\n" + "".join(
    "## Section %d\nA long **bold** Unicode строка with wrapped text and [[links]].\n\n" % n
    for n in range(500)
)


def run(binary, out, trial, mode):
    root = out / ("trial-%d" % trial) / mode
    vault, data = root / "vault", root / "data"
    vault.mkdir(parents=True, exist_ok=True)
    data.mkdir(parents=True, exist_ok=True)
    note = vault / "scroll.md"
    note.write_text(TEXT, encoding="utf-8")
    digest = hashlib.sha256(note.read_bytes()).hexdigest()
    replay, metrics, log = (root / name for name in ("replay.json", "metrics.json", "process.log"))
    env = dict(os.environ)
    env["AKULA_EXPERIMENTAL_WARP_MAP"] = mode
    env["AKULA_EXPERIMENTAL_FRAME_SAMPLES"] = "1"
    env["AKULA_EXPERIMENTAL_FRAME_STAGES"] = "1"
    env["AKULA_VELLO_SINGLE_RECT"] = "1"
    command = [str(binary), "--vault", str(vault), "--data-dir", str(data),
               "--open", str(note), "--threads", "2", "--size", "1200x760",
               "--replay-scroll", str(replay), "--metrics", str(metrics)]
    witness, peak = None, 0
    with log.open("wb") as stream:
        child = subprocess.Popen(command, env=env, stdout=stream, stderr=subprocess.STDOUT)
        started = time.monotonic()
        try:
            while child.poll() is None:
                witness = windows_gui_witness(child.pid) or witness
                peak = max(peak, windows_peak_rss_kib(child.pid))
                if time.monotonic() - started > 75:
                    raise TimeoutError("Windows replay timed out")
                time.sleep(0.05)
            if child.returncode:
                raise RuntimeError("GUI exit %d; inspect %s" % (child.returncode, log))
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=5)
    if not witness or not witness.get("uncloaked") or not witness.get("title_akula") or peak <= 0:
        raise RuntimeError("Missing visible HWND/PID or peak RSS")
    marker = "AKULA_WARP_MAP_PROBE requested=%s actual=%s driver=warp strict=1" % (mode, mode)
    if log.read_text(encoding="utf-8", errors="replace").count(marker) != 1:
        raise RuntimeError("Missing unique positive exact-WARP/map witness: %s" % log)
    rp = json.loads(replay.read_text(encoding="utf-8"))
    mt = json.loads(metrics.read_text(encoding="utf-8"))
    stages = rp.get("stageSamples")
    if not isinstance(stages, list) or len(stages) != rp.get("frames") or len(stages) < 20:
        raise RuntimeError("Unaligned or insufficient replay-only frame stages")
    for item in stages:
        for field in ("sceneMs", "rendererDrawMs", "outputPresentMs", "velloMs", "scrollShiftMs", "outputMapMs", "outputUnmapMs", "outputCopySubmitMs", "outputPresentCallMs"):
            if not isinstance(item.get(field), (int, float)) or not 0 <= item[field] < 100000:
                raise ValueError("Invalid frame stage " + field)
    for item in stages:
        for field in ("outputCopyBytes", "outputCopyRects", "outputMapCalls"):
            if not isinstance(item.get(field), int) or item[field] < 0:
                raise ValueError("Invalid native output counter: " + field)
    if not any(item["outputCopyBytes"] > 0 and item["outputCopyRects"] > 0
               and item["outputMapCalls"] > 0 for item in stages):
        raise RuntimeError("No native staging-map and copy witness")
    row = dict(trial=trial, mode=mode, note_sha256=digest, exe_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
               frames=len(stages), rss_kib=peak, visible_window=witness, replay=rp,
               output_present_p95_ms=percentile([s["outputPresentMs"] for s in stages], .95),
               renderer_draw_p95_ms=percentile([s["rendererDrawMs"] for s in stages], .95),
               scroll_shift_p95_ms=percentile([s["scrollShiftMs"] for s in stages], .95),
               map_p95_ms=percentile([s["outputMapMs"] for s in stages], .95),
               unmap_p95_ms=percentile([s["outputUnmapMs"] for s in stages], .95),
               copy_submit_p95_ms=percentile([s["outputCopySubmitMs"] for s in stages], .95),
               present_call_p95_ms=percentile([s["outputPresentCallMs"] for s in stages], .95),
               copy_bytes=sum(s["outputCopyBytes"] for s in stages),
               copy_rects=sum(s["outputCopyRects"] for s in stages))
    # A p95 alone hides rare multi-millisecond Map stalls. Report p99,
    # maxima and exact pixel workloads without filtering unusual frames.
    row.update(
        map_p99_ms=percentile([s["outputMapMs"] for s in stages], .99),
        map_max_ms=max(s["outputMapMs"] for s in stages),
        map_over_1ms=sum(s["outputMapMs"] > 1.0 for s in stages),
        renderer_draw_p99_ms=percentile([s["rendererDrawMs"] for s in stages], .99),
        raster_pixels=sum(s["rasterPixels"] for s in stages),
        shifted_pixels=sum(s["shiftedPixels"] for s in stages),
        map_calls=sum(s["outputMapCalls"] for s in stages),
    )
    (root / "sample.json").write_text(json.dumps(row, indent=2, ensure_ascii=False), encoding="utf-8")
    print("AKULA_WARP_MAP_SAMPLE trial=%d mode=%s frames=%d output_p95_ms=%.3f rss_kib=%d" %
          (trial, mode, len(stages), row["output_present_p95_ms"], peak), flush=True)
    return row


def percentile(values, portion):
    ordered = sorted(values)
    return ordered[round((len(ordered)-1)*portion)]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--out", required=True, type=Path)
    args = p.parse_args()
    if os.name != "nt":
        raise RuntimeError("Native Windows required")
    binary = args.binary.resolve(strict=True)
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    rows = []
    for trial in TRIALS:
        for mode in (MODES if trial % 2 else tuple(reversed(MODES))):
            rows.append(run(binary, out, trial, mode))
    if len(rows) != len(TRIALS) * len(MODES) or len({r["note_sha256"] for r in rows}) != 1 or len({r["exe_sha256"] for r in rows}) != 1:
        raise RuntimeError("Incomplete or mismatched twelve-process A/B")
    by_pair = {(r["trial"], r["mode"]): r for r in rows}
    if len(by_pair) != len(rows):
        raise RuntimeError("Duplicate trial/mode rather than independent measurements")
    # A previous WRITE process rasterized 13.6 million pixels versus 2.76
    # million in its partner. Such a pair is not valid performance evidence.
    for trial in TRIALS:
        write, readable = (by_pair[(trial, mode)] for mode in MODES)
        for metric in ("raster_pixels", "shifted_pixels", "copy_bytes"):
            left, right = write[metric], readable[metric]
            if max(left, right) == 0 or abs(left - right) / max(left, right) > .05:
                raise RuntimeError(
                    "Unmatched trial %s %s: WRITE=%s READ_WRITE=%s"
                    % (trial, metric, left, right)
                )
    lines = ["# WARP-MAP-01 — developer-only exact WARP 6 alternating pairs", "",
             "Real visible Win32 GUI and strict WARP on a Windows CI runner. Measurements are CPU",
             "work, not physical display scanout. CopySubresourceRegion is measured only",
             "for CPU submission, not GPU completion. Present1 may wait for vsync.", "",
             "| Mode | Map p95 | Unmap p95 | Copy call p95 | Present1 call p95 | Host present p95 | Draw p95 | Scroll shift p95 | Copy MiB | RSS MiB |",
             "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for mode in MODES:
        subset = [r for r in rows if r["mode"] == mode]
        lines.append("| %s | %.3f | %.3f | %.3f | %.3f | %.3f | %.3f | %.3f | %.1f | %.1f |" %
            (mode, median(r["map_p95_ms"] for r in subset),
             median(r["unmap_p95_ms"] for r in subset),
             median(r["copy_submit_p95_ms"] for r in subset),
             median(r["present_call_p95_ms"] for r in subset),
             median(r["output_present_p95_ms"] for r in subset),
             median(r["renderer_draw_p95_ms"] for r in subset),
             median(r["scroll_shift_p95_ms"] for r in subset),
             median(r["copy_bytes"] for r in subset)/1048576,
             median(r["rss_kib"] for r in subset)/1024))
    lines.extend([
        "",
        "## Paired frame tails and workloads (unfiltered)",
        "",
        "p99 and maxima expose stalls hidden by p95. All pairs are validated for",
        "rasterized, shifted and copied pixel counts within 5% before reporting.",
        "A smaller CPU call time is not proof of better physical display latency.",
        "",
        "| Trial | WRITE Map p99 ms | READ_WRITE Map p99 ms | WRITE Map max ms | READ_WRITE Map max ms | WRITE Map>1ms | READ_WRITE Map>1ms | Shift p95 WRITE ms | Shift p95 READ_WRITE ms | Raster pixels WRITE | Raster pixels READ_WRITE |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ])
    for trial in TRIALS:
        write, readable = (by_pair[(trial, mode)] for mode in MODES)
        lines.append("| %d | %.4f | %.4f | %.4f | %.4f | %d | %d | %.4f | %.4f | %d | %d |" % (
            trial, write["map_p99_ms"], readable["map_p99_ms"],
            write["map_max_ms"], readable["map_max_ms"],
            write["map_over_1ms"], readable["map_over_1ms"],
            write["scroll_shift_p95_ms"], readable["scroll_shift_p95_ms"],
            write["raster_pixels"], readable["raster_pixels"],
        ))
    (out / "matrix.json").write_text(json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf-8")
    (out / "summary.md").write_text("\n".join(lines)+"\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print("AKULA_WARP_MAP_ERROR %s: %s" % (type(exc).__name__, exc), file=sys.stderr)
        sys.exit(1)
