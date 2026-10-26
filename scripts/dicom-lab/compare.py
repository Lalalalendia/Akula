#!/usr/bin/env python3
"""Compare raw decoded bytes against independently verified synthetic source pixels."""
import csv
import json
import pathlib
import re
import subprocess
import time
from collections import Counter

root = pathlib.Path(__file__).resolve().parent
cases = json.loads((root / "manifest.json").read_text())["files"]
out = root / "results"
out.mkdir(exist_ok=True)
names = {
    "dicom-rs": "akula-dicom-rs-probe",
    "dcmfx": "akula-dcmfx-probe",
    "toolkit": "akula-toolkit-probe",
}
lossy = {"1.2.840.10008.1.2.4.50"}
rows = []
for name, bname in names.items():
    exe = root / name / "target" / "release" / bname
    for case in cases:
        if not exe.is_file():
            rows.append([name, case["file"], "NOT_COMPILED", "", "", "", "binary missing"])
            continue
        start = time.monotonic()
        try:
            p = subprocess.run(
                [str(exe), str(root / "fixtures" / case["file"])],
                capture_output=True, text=True, timeout=30, check=False
            )
            elapsed = round((time.monotonic() - start) * 1000, 3)
            detail = (p.stdout + "\n" + p.stderr).strip().replace("\n", " | ")[:1000]
            match = re.search(r"raw_sha256=([0-9a-f]{64})\b", p.stdout)
            actual = match.group(1) if match else ""
            decoded = p.returncode == 0 and "status=decoded" in p.stdout
            if case["expect"] == "reject":
                status = "UNEXPECTED_ACCEPTANCE" if decoded else "EXPECTED_REJECTION"
            elif not decoded:
                status = "DECODE_ERROR"
            elif case.get("transfer_syntax") in lossy:
                status = "LOSSY_DECODE_ONLY"
            elif actual == case.get("pixel_sha256"):
                status = "PIXEL_MATCH"
            else:
                status = "PIXEL_MISMATCH"
            rows.append([name, case["file"], status, elapsed, p.returncode, actual, detail])
        except Exception as exc:
            rows.append([name, case["file"], "ERROR", "", "", "", repr(exc)])
with (out / "results.csv").open("w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["library", "file", "status", "runtime_ms", "exit_code", "raw_sha256", "detail"])
    w.writerows(rows)
for row in rows:
    print("PROBE", row[0], row[1], row[2], row[3])
for name in names:
    print("RESULT_SUMMARY", name, dict(Counter(row[2] for row in rows if row[0] == name)))
(out / "limitations.txt").write_text(
    "Synthetic data only. 8 lossless raw-pixel hashes checked per Rust decoder; "
    "lossy JPEG decoded but fidelity not measured. The 8-bit output/display "
    "windows differ by library, so rendered image parity is NOT established. "
    "Micro-timings include process startup and lack repeated sampling. "
    "No peak RSS, DICOMDIR, real CD/USB, 100 MB series, or clinical qualification.\n"
)
