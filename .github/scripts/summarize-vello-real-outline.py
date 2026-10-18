#!/usr/bin/env python3
"""Diagnostic-only summary of real glifo outline mask slot/order invariance.

Each source path comes from the pending outline glyphs of Akula's real editor.
A probe compares normalized mask pixels before tint. Zero errors does not
prove the full atlas batch/cache path correct; nonzero errors locate a
reproducible real-path rasterization discrepancy.
"""
from __future__ import annotations

import collections
import pathlib
import re
import sys

LINE = re.compile(
    r"VELLO_REAL_OUTLINE sample=(\d+) page=(\d+) cmd=(\d+) "
    r"elements=(\d+) variant=(\S+) .*?"
    r"changed=(\d+) peak=(\d+) first=(.*)"
)
KINDS = (
    "slot_1",
    "slot_2",
    "slot_3",
    "slot_4",
    "slot_5",
    "slot_6",
    "slot_7",
    "batch_before",
    "batch_after",
)


def summarize(folder: pathlib.Path) -> str:
    logs = sorted(folder.glob("outline-mask-*.log"))
    if len(logs) != 8:
        raise RuntimeError(f"expected eight independent UI process logs; got {len(logs)}")
    per_variant: dict[str, list[tuple[int, int, str, int]]] = collections.defaultdict(list)
    red_count = 0
    all_samples: set[tuple[str, str]] = set()
    for log in logs:
        txt = log.read_text(errors="replace")
        if "VELLO_GUARD_PARITY_FAILURE case=cover_1x" in txt:
            red_count += 1
        rows = LINE.findall(txt)
        if not rows:
            raise RuntimeError(f"{log.name} did not capture any real glifo outline")
        for sample, page, cmd, elements, variant, changed, peak, first in rows:
            if variant not in KINDS:
                raise RuntimeError(f"{log.name}: unknown variant {variant}")
            per_variant[variant].append((int(changed), int(peak), first, int(elements)))
            all_samples.add((log.name, sample))
        counts = collections.Counter(sample for sample, _, _, _, _, _, _, _ in rows)
        if any(count != len(KINDS) for count in counts.values()):
            raise RuntimeError(f"{log.name}: not every shape got 7 slot + 2 batch comparisons: {counts}")

    lines = [
        "# Real outline glyph atlas slot and batch-order diagnostic",
        "",
        f"Real guard96 UI processes: **8**, strict parity red: **{red_count}/8**.",
        f"Real glyph outline instances probed: **{len(all_samples)}**. ",
        "No synthetic outlines were substituted.",
        "",
        "| Variant | Comparisons | Exact mismatches | Max changed pixels | Peak channel |",
        "|---|---:|---:|---:|---:|",
    ]
    differing = []
    for variant in KINDS:
        rows = per_variant[variant]
        if not rows:
            raise RuntimeError(f"missing variant {variant}")
        red = [r for r in rows if r[0] > 0]
        lines.append(
            f"| {variant} | {len(rows)} | {len(red)} | "
            f"{max(r[0] for r in rows)} | {max(r[1] for r in rows)} |"
        )
        for changed, peak, first, n_elements in red[:7]:
            differing.append(
                f"- {variant}: {changed} changed, peak {peak}/255, "
                f"{n_elements} real contour elements, first={first}"
            )
    lines += [
        "",
        "## Interpretation",
        "",
        "Comparisons normalize the same *real* glyph path, scale and fractional",
        "phase to seven integer atlas slots; the batch tests draw a second real",
        "glyph at a non-overlapping position before/after the target.",
        "All comparisons are against the same isolated outline mask.",
        "",
        "These comparisons are independent diagnostics, **not** a relaxed",
        "fidelity gate. Original strict guard96 remains required for release.",
    ]
    if differing:
        lines += ["", "## First mismatches", "", *differing[:24]]
    else:
        lines += [
            "",
            "**No mismatch seen in this constrained real-path test.**",
            "That would rule out simple integer slot translation / disjoint",
            "two-draw order as sufficient causes, but not full atlas batching,",
            "dirty crop, cache reuse, clipping, or long multi-glyph interactions.",
        ]
    report = "\n".join(lines) + "\n"
    (folder / "real-outline-summary.md").write_text(report)
    return report


if __name__ == "__main__":
    print(summarize(pathlib.Path(sys.argv[1])))
