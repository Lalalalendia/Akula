#!/usr/bin/env python3
"""Process-isolated warm-cache wall and peak-RSS benchmark; not streaming latency."""
import csv
import json
import pathlib
import re
import statistics
import subprocess
import tempfile
import time

root = pathlib.Path(__file__).resolve().parent
out = root / "results"
out.mkdir(exist_ok=True)
case = json.loads((root / "stress_manifest.json").read_text())
fixture = root / "fixtures" / case["file"]
assert fixture.stat().st_size == case["size"]
names = {
    "dicom-rs": "akula-dicom-rs-probe",
    "dcmfx": "akula-dcmfx-probe",
    "toolkit": "akula-toolkit-probe",
}
rows = []
for name, binary_name in names.items():
    exe = root / name / "target" / "release" / binary_name
    for iteration in range(4):
        is_warmup = iteration == 0
        rss_file = out / ("time_" + name + "_" + str(iteration) + ".txt")
        start = time.perf_counter()
        try:
            p = subprocess.run(["/usr/bin/time", "-f", "%M", "-o", str(rss_file),
                                str(exe), str(fixture)],
                               capture_output=True, text=True, timeout=180, check=False)
            wall_ms = round((time.perf_counter() - start) * 1000, 3)
            rss_kib = int(rss_file.read_text().strip())
            digest = re.search(r"raw_sha256=([a-f0-9]{64})\b", p.stdout)
            frames = re.search(r"frames=(\d+)", p.stdout)
            if p.returncode != 0:
                status = "DECODE_ERROR"
            elif not digest or not frames:
                status = "NO_VALID_RESULT"
            elif int(frames.group(1)) != case["frames"]:
                status = "FRAME_MISMATCH"
            elif digest.group(1) != case["pixel_sha256"]:
                status = "PIXEL_MISMATCH"
            else:
                status = "PIXEL_MATCH"
            detail = (p.stdout + "\n" + p.stderr).strip().replace("\n", " | ")[:750]
        except Exception as exc:
            status = "ERROR"
            detail = repr(exc)
            wall_ms = ""
            rss_kib = ""
        rows.append(dict(library=name, trial=iteration,
                         phase="warmup" if is_warmup else "repeated",
                         status=status, wall_ms=wall_ms,
                         max_rss_kib=rss_kib, detail=detail))
        print("STRESS", name, iteration, status, "wall_ms", wall_ms, "rss_kib", rss_kib, flush=True)
fields = ["library", "trial", "phase", "status", "wall_ms", "max_rss_kib", "detail"]
with (out / "stress_results.csv").open("w", newline="") as f:
    w = csv.DictWriter(f, fieldnames=fields)
    w.writeheader()
    w.writerows(rows)
for name in names:
    trials = [r for r in rows if r["library"] == name and r["phase"] == "repeated"]
    if len(trials) == 3 and all(r["status"] == "PIXEL_MATCH" for r in trials):
        print("STRESS_MEDIAN", name,
              "wall_ms", round(statistics.median(r["wall_ms"] for r in trials), 1),
              "peak_rss_mib", round(max(r["max_rss_kib"] for r in trials) / 1024, 1))
    else:
        print("STRESS_INCOMPLETE", name, [r["status"] for r in trials])
