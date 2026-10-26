#!/usr/bin/env python3
"""Random-access single-frame probes, not full-volume visualization."""
import csv
import json
import pathlib
import re
import statistics
import subprocess
import time
from collections import Counter

root = pathlib.Path(__file__).resolve().parent
out = root / "results"
out.mkdir(exist_ok=True)
case = json.loads((root / "stress_manifest.json").read_text())
expected = case["frame_pixel_sha256"]
input_file = str(root / "fixtures" / case["file"])
names = ["dicom-rs", "dcmfx", "toolkit"]
frame_ids = [0, 99, 199]
rows = []
for name in names:
    exe = root / name / "target" / "release" / "slice_probe"
    trials = [(0, True)] + [(i, False) for i in frame_ids for _ in range(3)]
    for trial, (frame, warmup) in enumerate(trials):
        mem_path = out / ("slice_mem_" + name + "_" + str(trial) + ".txt")
        started = time.perf_counter()
        try:
            p = subprocess.run(
                ["/usr/bin/time", "-f", "%M", "-o", str(mem_path),
                 str(exe), input_file, str(frame)],
                capture_output=True, text=True, check=False, timeout=90
            )
            wall = round((time.perf_counter() - started) * 1000, 3)
            peak = int(mem_path.read_text().strip())
            actual = re.search(r"raw_sha256=([a-f0-9]{64})\b", p.stdout)
            frm = re.search(r"frames=(\d+)", p.stdout)
            siz = re.search(r"pixel_bytes=(\d+)", p.stdout)
            ok = p.returncode == 0 and actual and frm and siz
            status = ("PIXEL_MATCH" if ok and actual.group(1)==expected
                      and int(frm.group(1))==1 and int(siz.group(1))==524288
                      else "PIXEL_MISMATCH" if ok else "ERROR")
            detail = (p.stdout + " " + p.stderr).replace("\n", " | ")[:700]
        except Exception as e:
            wall, peak, status, detail = "", "", "ERROR", repr(e)
        rows.append(dict(library=name, trial=trial, frame=frame,
                         warmup=warmup, status=status,
                         wall_ms=wall, max_rss_kib=peak, detail=detail))
        print("SLICE", name, frame, "warmup" if warmup else "repeat",
              status, wall, peak, flush=True)
with (out / "slice_results.csv").open("w", newline="") as f:
    fieldnames = ["library", "trial", "frame", "warmup", "status",
                  "wall_ms", "max_rss_kib", "detail"]
    w = csv.DictWriter(f, fieldnames=fieldnames)
    w.writeheader()
    w.writerows(rows)
for name in names:
    checks = [r for r in rows if r["library"]==name and not r["warmup"]]
    print("SLICE_SUMMARY", name, Counter(x["status"] for x in checks))
    for frame in frame_ids:
        data = [r for r in checks if r["frame"]==frame and r["status"]=="PIXEL_MATCH"]
        if len(data)==3:
            print("SLICE_MEDIAN", name, frame,
                  round(statistics.median(x["wall_ms"] for x in data), 3),
                  round(max(x["max_rss_kib"] for x in data)/1024, 2))
