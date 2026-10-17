#!/usr/bin/env python3
"""Summarize Akula's paired release-mode Vello trace without additive CPU claims.

Reads a Chrome trace emitted by the existing aq_trace library and the
untraced/traced benchmark logs. Reports main-thread coarse/fine wall phases;
worker-local spans overlap and must NOT be summed into wall time.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

CASES = ("text_1x", "text_175", "cover_1x", "cover_175")
MODES = ("serial", "parallel")
PHASES = {
    "vello: render_with": "render_with",
    "vello: подготовка глифов": "glyph_prepare",
    "vello: coarse bucket": "coarse_bucket",
    "vello: раскладка по строкам": "coarse_bucket",
    "vello: fine wall": "fine_wall",
    "vello: чистка кэшей": "cache_maintenance",
}


def percentile(values: list[float], portion: float) -> float:
    if not values:
        return 0.0
    data = sorted(values)
    pos = (len(data) - 1) * portion
    left = int(pos)
    right = min(left + 1, len(data) - 1)
    return data[left] + (data[right] - data[left]) * (pos - left)


def summaries(log: Path) -> dict[tuple[str, str], dict[str, str]]:
    found = {}
    for line in log.read_text().splitlines():
        if "VELLO_RELEASE case=" not in line:
            continue
        fields = dict(
            part.split("=", 1)
            for part in line.split("VELLO_RELEASE ", 1)[1].split()
            if "=" in part
        )
        key = (fields["case"], fields["mode"])
        if key in found:
            raise ValueError(f"duplicate profile summary: {key}")
        found[key] = fields
    expected = {(case, mode) for case in CASES for mode in MODES}
    if set(found) != expected:
        raise ValueError(f"incomplete release matrix: missing={expected-set(found)} extra={set(found)-expected}")
    for key, row in found.items():
        if row.get("frames") != "31" or row.get("parity") != "4":
            raise ValueError(f"incomplete frame or pixel coverage at {key}: {row}")
    return found


def trace_phases(path: Path) -> dict[tuple[str, str], dict[str, tuple[float, float, int]]]:
    data = json.loads(path.read_text())
    events = data["traceEvents"]
    marks = [e for e in events if e.get("ph") == "i"]
    spans = [e for e in events if e.get("ph") == "X" and e.get("dur", 0) >= 0]
    output = {}
    for case in CASES:
        start_name = f"vello_{case}_begin"
        end_name = f"vello_{case}_end"
        starts = [float(e["ts"]) for e in marks if e.get("name") == start_name]
        ends = [float(e["ts"]) for e in marks if e.get("name") == end_name]
        if len(starts) != 1 or len(ends) != 1 or starts[0] >= ends[0]:
            raise ValueError(f"missing or inverted trace markers for {case}")
        begin, end = starts[0], ends[0]
        for mode in MODES:
            parent_name = f"bench: {mode} frame"
            frames = sorted(
                (e for e in spans if e.get("name") == parent_name and
                 begin <= float(e["ts"]) < end),
                key=lambda e: e["ts"],
            )
            if len(frames) != 32:
                raise ValueError(f"{case}/{mode}: expected 32 traced frames, got {len(frames)}")
            measurements = {phase: [] for phase in set(PHASES.values())}
            active_frames = 0
            # Skip step 0, which is intentionally a full initial render.
            for frame in frames[1:]:
                lo = float(frame["ts"])
                hi = lo + float(frame["dur"])
                phases = {}
                for e in spans:
                    name = PHASES.get(e.get("name"))
                    if name is None:
                        continue
                    ts = float(e["ts"])
                    if lo <= ts and ts + float(e["dur"]) <= hi + 0.01:
                        phases[name] = phases.get(name, 0.0) + float(e["dur"])
                if phases.get("render_with", 0) > 0:
                    active_frames += 1
                    for phase, values in measurements.items():
                        values.append(phases.get(phase, 0.0))
            if active_frames < 18:
                raise ValueError(f"{case}/{mode}: only {active_frames} traced raster frames")
            output[(case, mode)] = {
                phase: (percentile(values, 0.5), percentile(values, 0.95), active_frames)
                for phase, values in measurements.items()
            }
    return output


def main() -> None:
    if len(sys.argv) != 4:
        raise SystemExit("usage: summarize-vello-profile.py trace.json untraced.log traced.log")
    trace, untraced, traced = map(Path, sys.argv[1:])
    base = summaries(untraced)
    _traced_log = summaries(traced)
    phases = trace_phases(trace)
    out = [
        "## Vello CPU — paired release-mode renderer attribution",
        "",
        "Akula native release test binary, same host, SSE4.2 rasterization,",
        "31 interactive partial frames per case, serial/2-worker ABBA,",
        "4 exact partial/full reference frames and strict serial/parallel",
        "pixel equality per case. Raw Chrome trace is retained as an artifact.",
        "",
        "### Untraced release-mode frame timings",
        "",
        "| Scenario | Workers | Frame p50/p95 (ms) | Encoding p50/p95 (ms) | Raster p50/p95 (ms) | Raster area p50 (%) |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for mode in MODES:
            row = base[(case, mode)]
            def ms(key: str) -> str:
                return f"{float(row[key])/1000:.3f}"
            out.append(
                f"| {case} | {0 if mode == 'serial' else 2} | "
                f"{ms('frame_p50_us')}/{ms('frame_p95_us')} | "
                f"{ms('encode_p50_us')}/{ms('encode_p95_us')} | "
                f"{ms('raster_p50_us')}/{ms('raster_p95_us')} | "
                f"{row['area_p50_pct']} |"
            )
    out.extend([
        "",
        "### Traced release-mode Vello phases",
        "",
        "Phase wall durations in microseconds. Parent render_with contains some",
        "children; do NOT add phases. Worker-local parallel vello:fine",
        "events overlap and are deliberately excluded. Tracing uses a mutex",
        "and adds overhead; compare optimization candidates using the untraced",
        "same-runner ABBA numbers above, not trace phase totals.",
        "",
        "| Scenario | Workers | render_with p50/p95 | Glyph prep p50/p95 | Coarse bucket p50/p95 | Fine wall p50/p95 |",
        "|---|---:|---:|---:|---:|---:|",
    ])
    for case in CASES:
        for mode in MODES:
            row = phases[(case, mode)]
            def display(key: str) -> str:
                a, b, _ = row[key]
                return f"{a:.1f}/{b:.1f}"
            out.append(
                f"| {case} | {0 if mode == 'serial' else 2} | "
                f"{display('render_with')} | {display('glyph_prepare')} | "
                f"{display('coarse_bucket')} | {display('fine_wall')} |"
            )
    out.extend([
        "",
        "### Trace sampling check",
        "",
        "| Scenario | Workers | Raster samples | Traced render_with p50 (µs) |",
        "|---|---:|---:|---:|",
    ])
    for case in CASES:
        for mode in MODES:
            values = phases[(case, mode)]["render_with"]
            out.append(f"| {case} | {0 if mode=='serial' else 2} | {values[2]} | {values[0]:.1f} |")
    out.append("")
    report = "\n".join(out)
    Path("vello-release-summary.md").write_text(report)
    print(report)


if __name__ == "__main__":
    main()
