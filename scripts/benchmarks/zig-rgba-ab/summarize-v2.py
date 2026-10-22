#!/usr/bin/env python3
"""Independent-process aggregate; never pool individual sample timing."""
import json
import statistics
import sys
from collections import defaultdict
from pathlib import Path

reports = [json.loads(Path(p).read_text()) for p in sys.argv[1:]]
assert len(reports) == 5
assert len({r["run"] for r in reports}) == 5
assert len({r["sha"] for r in reports}) == 1
groups = defaultdict(list)
for report in reports:
    assert len(report["checks"]) == 45
    assert all(x["parity"] for x in report["checks"])
    for x in report["measurements"]:
        assert x["samples"] == 10
        groups[(tuple(x["size"]), x["variant"])].append(x["median_ms"])
aggregated, ratios = [], []
for (size, variant), values in sorted(groups.items()):
    assert len(values) == 5
    aggregated.append({"size": list(size), "variant": variant,
                       "median_of_process_medians_ms": statistics.median(values),
                       "min_process_ms": min(values), "max_process_ms": max(values),
                       "process_medians_ms": values})
    print(f"{size[0]}x{size[1]} {variant:<24} "
          f"{statistics.median(values):7.4f} ms; range {min(values):.4f}-{max(values):.4f}")
for size in ((1280, 884), (4096, 2160)):
    for label, a, b in (
        ("generic_zig_vs_rust_packed", "rust_generic_packed", "zig_generic_packed"),
        ("native_zig_vs_rust_packed", "rust_native_packed", "zig_native_packed"),
        ("unaligned_vs_rust_packed", "rust_generic_packed", "rust_generic_unaligned"),
        ("row_vs_rust_packed", "rust_generic_packed", "rust_generic_row"),
        ("generic_zig_vs_rust_row", "rust_generic_row", "zig_generic_packed"),
        ("native_zig_vs_rust_row", "rust_native_row", "zig_native_packed"),
    ):
        av, bv = groups[(size, a)], groups[(size, b)]
        relative = [x / y for x, y in zip(av, bv)]
        ratios.append({"size": list(size), "comparison": label,
                       "median_ratio": statistics.median(relative),
                       "process_ratios": relative})
        print(f"RATIO {size[0]}x{size[1]} {label}: "
              f"{statistics.median(relative):.4f}x"
              f" per process {[round(r,3) for r in relative]}")
Path(".rgba-comparison-v2/summary.json").write_text(json.dumps({
    "sha": reports[0]["sha"], "independent_processes": 5,
    "all_parity_passed": True, "aggregated": aggregated, "ratios": ratios
}, indent=2))
