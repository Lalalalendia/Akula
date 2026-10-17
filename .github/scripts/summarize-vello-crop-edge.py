#!/usr/bin/env python3
"""Validate full-frame coverage and compare crop-cut offsets for 8-strip Vello."""

import argparse
import re
import statistics
from pathlib import Path

BIASES = (-48, -24, 0, 24, 48)
LINE = re.compile(
    r"VELLO_CROP_EDGE_BIAS bias=(-?\d+) first_cut_y=(\d+) "
    r"frame=3 strips=8 changed=(\d+) severe_gt1=(\d+) "
    r"max_channel=(\d+) severe_bbox="
)


def parse(log: Path) -> dict[int, dict]:
    text = log.read_text(encoding="utf-8")
    if "test result: ok." not in text:
        raise ValueError(f"{log}: focused Rust scene failed")
    data = {}
    for line in text.splitlines():
        found = LINE.search(line)
        if found is None:
            continue
        bias, y, changed, severe, maximum = map(int, found.groups())
        if bias in data:
            raise ValueError(f"{log}: duplicated cut bias {bias}")
        if y != 131 + bias or severe > changed or not 0 <= maximum <= 255:
            raise ValueError(f"{log}: invalid observation for bias={bias}")
        data[bias] = {"changed": changed, "severe": severe, "maximum": maximum}
    if set(data) != set(BIASES):
        raise ValueError(f"{log}: missing {sorted(set(BIASES)-set(data))}")
    return data


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("logs", nargs=12, type=Path)
    ap.add_argument("--output", type=Path, required=True)
    args = ap.parse_args()
    runs = [parse(x) for x in args.logs]
    lines = [
        "# Vello cropped UI-layer first-band cutoff sweep",
        "",
        "12 independent release-mode Linux runs of the same real Akula "
        "cover@1.75x UI at step 3, eight disjoint strips. "
        "The first band cutoff is moved around the rounded focus-mode control "
        "without changing total raster coverage.",
        "",
        "| Cut bias (px) | First boundary Y | Failures >1 LSB | Median severe pixels | Peak channel across trials |",
        "|---:|---:|---:|---:|---:|",
    ]
    for bias in BIASES:
        rows = [run[bias] for run in runs]
        fails = sum(item["severe"] > 0 for item in rows)
        median = statistics.median(item["severe"] for item in rows)
        peak = max(item["maximum"] for item in rows)
        lines.append(f"| {bias:+} | {131 + bias} | {fails}/12 | {median:.0f} | {peak}/255 |")
    lines += [
        "",
        "Do not interpret a green diagnostic run as renderer correctness: "
        "the 1/255 full-reference gate in the parent fragmented-frame suite "
        "remains authoritative. If moving the boundary past the button "
        "eliminates the large mismatch, inspect clipped rounded paths, "
        "group masks and intermediate layer bounds before touching threads.",
        "Source PNG pairs of every >1-LSB trial are archived for review.",
        "",
    ]
    report = "\n".join(lines)
    args.output.write_text(report, encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
