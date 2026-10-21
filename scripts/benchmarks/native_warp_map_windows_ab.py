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
    for trial in (1, 2, 3):
        for mode in (MODES if trial % 2 else tuple(reversed(MODES))):
            rows.append(run(binary, out, trial, mode))
    if len(rows) != 6 or len({r["note_sha256"] for r in rows}) != 1 or len({r["exe_sha256"] for r in rows}) != 1:
        raise RuntimeError("Incomplete or mismatched six-process A/B")
    lines = ["# WARP-MAP-01 — developer-only exact WARP 3×AB/BA", "",
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
    (out / "matrix.json").write_text(json.dumps(rows, indent=2, ensure_ascii=False), encoding="utf-8")
    (out / "summary.md").write_text("\n".join(lines)+"\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print("AKULA_WARP_MAP_ERROR %s: %s" % (type(exc).__name__, exc), file=sys.stderr)
        sys.exit(1)
