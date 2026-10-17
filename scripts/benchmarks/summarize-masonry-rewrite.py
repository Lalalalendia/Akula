#!/usr/bin/env python3
"""Analyze Masonry rewrite passes inside Akula note-page RenderRoot::edit.

Input is aq_trace's Chrome trace JSON from the opt-in native note matrix.
The trace changes test overhead; use it for attribution, not speedup claims.
"""
import json
import math
import sys
from collections import defaultdict
from pathlib import Path

SIZES = (64, 512, 4096)
PASSES = (
    "run_mutate_pass",
    "run_action_pass",
    "run_update_widget_tree_pass",
    "run_update_disabled_pass",
    "run_update_stashed_pass",
    "run_update_focusable_pass",
    "run_update_focus_pass",
    "run_layout_pass",
    "run_update_scroll_pass",
    "run_compose_pass",
    "run_update_pointer_pass",
    "run_update_props_pass",
)
PREFIX = "masonry: "
ROOT_ZONE = "masonry: rewrite passes"
PAGE_BEGIN = "akula: note page edit begin"
PAGE_END = "akula: note page edit end"
PARAGRAPH_COUNTERS = (
    "akula: paragraph built",
    "akula: paragraph reused",
    "akula: paragraph skipped",
    "akula: paragraph rebroken",
    "akula: paragraph shape micros",
    "akula: paragraph rebreak micros",
)


def p95(values):
    if not values:
        raise AssertionError("Empty profile series")
    ordered = sorted(values)
    return ordered[max(0, math.ceil(0.95 * len(ordered)) - 1)]


def one_marker(events, name):
    found = [e for e in events if e.get("ph") == "i" and e.get("name") == name]
    if len(found) != 1:
        raise AssertionError(f"Expected one {name!r} marker; got {len(found)}")
    return found[0]


def profile(trace, expected_cycles=16):
    events = trace["traceEvents"]
    results = []
    for size in SIZES:
        start = one_marker(events, f"akula: matrix {size} begin")
        stop = one_marker(events, f"akula: matrix {size} end")
        tid = start["tid"]
        if stop["tid"] != tid or stop["ts"] <= start["ts"]:
            raise AssertionError(f"Bad matrix thread or window for {size}")
        marks = sorted(
            (e for e in events
             if e.get("ph") == "i" and e.get("tid") == tid
             and start["ts"] <= e["ts"] <= stop["ts"]
             and e["name"] in (PAGE_BEGIN, PAGE_END)),
            key=lambda e: e["ts"],
        )
        if len(marks) != expected_cycles * 2:
            raise AssertionError(
                f"{size}: expected {expected_cycles} page edits, found {len(marks) // 2}"
            )
        pairs = []
        for i in range(0, len(marks), 2):
            a, b = marks[i:i+2]
            if a["name"] != PAGE_BEGIN or b["name"] != PAGE_END or a["ts"] >= b["ts"]:
                raise AssertionError(f"Unbalanced page edit boundaries for {size}")
            pairs.append((a["ts"], b["ts"]))
        own_zones = sorted(
            (e for e in events if e.get("ph") == "X" and e.get("tid") == tid
             and start["ts"] <= e["ts"] <= stop["ts"]),
            key=lambda e: e["ts"],
        )
        own_counters = sorted(
            (e for e in events if e.get("ph") == "C" and e.get("tid") == tid
             and e.get("name") in PARAGRAPH_COUNTERS
             and start["ts"] <= e["ts"] <= stop["ts"]),
            key=lambda e: e["ts"],
        )
        samples = []
        for a, b in pairs:
            timings = defaultdict(float)
            counts = defaultdict(int)
            for e in own_zones:
                at = e["ts"]
                if at < a:
                    continue
                if at > b:
                    break
                if at + e["dur"] > b + 0.5:
                    continue
                name = e["name"]
                if name == ROOT_ZONE or (name.startswith(PREFIX) and name[len(PREFIX):] in PASSES):
                    timings[name] += e["dur"] / 1000.0
                    counts[name] += 1
            if counts[ROOT_ZONE] != 1:
                raise AssertionError(f"{size}: expected one rewrite per edit; got {counts[ROOT_ZONE]}")
            counter_count = defaultdict(int)
            for e in own_counters:
                at = e["ts"]
                if at < a:
                    continue
                if at > b:
                    break
                timings[e["name"]] += e["args"]["value"]
                counter_count[e["name"]] += 1
            if any(counter_count[name] == 0 for name in PARAGRAPH_COUNTERS):
                raise AssertionError(f"{size}: missing per-paragraph trace counters in a page edit")
            timings["full_edit"] = (b - a) / 1000.0
            timings["outside_rewrite"] = max(0.0, timings["full_edit"] - timings[ROOT_ZONE])
            samples.append(timings)
        result = {
            "size_kib": size,
            "cycles": len(samples),
            "p95_ms": {field: round(p95([s.get(field, 0.0) for s in samples]), 4)
                       for field in ("full_edit", ROOT_ZONE, "outside_rewrite")
                       + tuple(PREFIX + name for name in PASSES)},
            "paragraph_p95": {field: round(p95([s.get(field, 0.0) for s in samples]), 3)
                              for field in PARAGRAPH_COUNTERS},
        }
        results.append(result)
    return results


def main():
    if len(sys.argv) not in (2, 3):
        raise SystemExit("usage: summarize-masonry-rewrite.py trace.json [cycles]")
    trace = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    results = profile(trace, int(sys.argv[2]) if len(sys.argv) == 3 else 16)
    headers = [
        ("full_edit", "RenderRoot::edit total"),
        (ROOT_ZONE, "Rewrite passes total"),
        ("outside_rewrite", "Outside rewrite passes"),
    ]
    headers.extend((PREFIX + x, x.removeprefix("run_")) for x in PASSES)
    print("## Masonry page-install pass attribution (p95 milliseconds)")
    print()
    print("Opt-in trace, Ubuntu debug profile. Trace synchronization adds overhead;")
    print("do not compare these numbers to untraced note latency or sum independent p95 values.")
    print()
    print("| Stage | 64 KiB | 512 KiB | 4096 KiB |")
    print("|---|---:|---:|---:|")
    for key, label in headers:
        values = " | ".join(f"{r['p95_ms'][key]:.3f}" for r in results)
        print(f"| {label} | {values} |")
    print()
    print("### Paragraph shaping and cache activity per page edit (p95)")
    print()
    print("| Metric | 64 KiB | 512 KiB | 4096 KiB |")
    print("|---|---:|---:|---:|")
    labels = (
        ("akula: paragraph built", "New layouts, paragraphs", 1),
        ("akula: paragraph reused", "Reused cached layouts, paragraphs", 1),
        ("akula: paragraph skipped", "Skipped offscreen paragraphs", 1),
        ("akula: paragraph rebroken", "Cached layouts rewrapped", 1),
        ("akula: paragraph shape micros", "Time building new layouts, ms", 1/1000),
        ("akula: paragraph rebreak micros", "Time rewrapping cached layouts, ms", 1/1000),
    )
    for key, label, scale in labels:
        values = " | ".join(f"{r['paragraph_p95'][key] * scale:.3f}" for r in results)
        print(f"| {label} | {values} |")
    print()
    print("Counter totals include all paragraph measurement invocations inside a single")
    print("page edit. Per-column p95s are independent; they cannot be added.")
    print("Each page edit has one verified rewrite zone and all six paragraph counters.")
    print("The full trace remains as an artifact for deeper inspection.")


if __name__ == "__main__":
    main()
