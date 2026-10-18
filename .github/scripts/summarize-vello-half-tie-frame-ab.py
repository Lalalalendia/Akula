#!/usr/bin/env python3
"""Compare actual full real-editor frame bytes before/after Vello coverage change.

Deterministic input fixtures: 250 paragraphs, cover and plain markdown, 1x and
1.75x, first and final scroll frames. This does NOT assert identity between
different rounding policies: it records exact magnitude / distribution.
"""
from __future__ import annotations

import argparse
from pathlib import Path

SCENES = {
    "text_1x": (1000, 600),
    "text_175": (1750, 1051),
    "cover_1x": (1000, 600),
    "cover_175": (1750, 1051),
}


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("baseline", type=Path)
    p.add_argument("patched", type=Path)
    p.add_argument("--output", type=Path, required=True)
    args = p.parse_args()
    lines = [
        "# Vello common half-tie: identical real-editor full-frame A/B",
        "",
        "Fixed font/render scene inputs; stored pixels are little-endian u32.",
        "Independent release binary runs on one CI runner. This is a diagnostic",
        "of global visual change, not proof of a safe patch for all fonts.",
        "",
        "| Scene | Step | Pixels | Changed | Fraction | >1/255 | >2/255 | Max channel delta |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    changed_total = 0
    gt1_total = 0
    pixels_total = 0
    maximum = 0
    for scene, (width, height) in SCENES.items():
        for step in (0, 31):
            name = f"{scene}-step{step:02}.u32le"
            pa, pb = args.baseline / name, args.patched / name
            a, b = pa.read_bytes(), pb.read_bytes()
            want = width * height * 4
            if len(a) != want or len(b) != want:
                raise SystemExit(f"Nonmatching frame sizes {name}: {len(a)}, {len(b)}, expected {want}")
            changed = gt1 = gt2 = peak = 0
            # Count pixels, not individual channels; do not materialize large
            # integer copies or reinterpretation-dependent byte orders.
            for i in range(0, want, 4):
                delta = max(abs(x - y) for x, y in zip(a[i:i+4], b[i:i+4]))
                peak = max(peak, delta)
                changed += delta > 0
                gt1 += delta > 1
                gt2 += delta > 2
            count = width * height
            pixels_total += count
            changed_total += changed
            gt1_total += gt1
            maximum = max(maximum, peak)
            lines.append(
                f"| {scene} | {step} | {count:,} | {changed:,} | "
                f"{100 * changed / count:.6f}% | {gt1:,} | {gt2:,} | {peak} |"
            )
    lines.extend([
        "",
        f"**All eight frames:** {changed_total:,} / {pixels_total:,} pixels changed, "
        f"{gt1_total:,} over 1/255, maximum channel delta {maximum}/255.",
        "",
        "An aggregate pixel diff can include differences in font atlas placement",
        "or UI determinism; check individual files if surprising. This audit is",
        "not a replacement for within-build full/partial strict parity.",
        "",
    ])
    text = "\n".join(lines)
    args.output.write_text(text, encoding="utf-8")
    print(text)


if __name__ == "__main__":
    main()
