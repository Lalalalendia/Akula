#!/usr/bin/env python3
"""Strictly validate independent same-run fixed-vs-adaptive scroll pipeline A/B."""
import argparse
import math
from pathlib import Path
from statistics import median

TRIALS = (1, 2, 3)
MODES = ("fixed2", "adaptive2")
CASES = ("plain_1x", "cover_175")
REQUIRED_FLOATS = (
    "total_p50_us", "total_p95_us", "total_sum_us",
    "input_p50_us", "input_p95_us", "input_sum_us",
    "scene_p50_us", "scene_p95_us", "scene_sum_us",
    "draw_p50_us", "draw_p95_us", "draw_sum_us",
)
REQUIRED_INTS = ("frames", "shifts", "full_frames", "small_frames",
                 "switches", "parity", "peak_kib")


def parse(path: Path, expected_mode: str) -> dict:
    if not path.is_file():
        raise ValueError(f"Missing separate-process log: {path}")
    rows = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "AKULA_SCROLL_PIPELINE case=" not in line:
            continue
        payload = line.split("AKULA_SCROLL_PIPELINE ", 1)[1]
        fields = dict(token.split("=", 1) for token in payload.split() if "=" in token)
        label, mode = fields.get("case"), fields.get("mode")
        if label not in CASES or mode != expected_mode or label in rows:
            raise ValueError(f"Unexpected or duplicate case/mode in {path}: {label}/{mode}")
        for field in REQUIRED_INTS:
            fields[field] = int(fields[field])
        for field in REQUIRED_FLOATS:
            fields[field] = float(fields[field])
            if not math.isfinite(fields[field]) or fields[field] <= 0:
                raise ValueError(f"Non-finite/non-positive {field} in {path}")
        if (fields["frames"] < 55 or fields["shifts"] < 45 or
                fields["full_frames"] < 6 or fields["small_frames"] < 30 or
                fields["parity"] != 6 or fields["peak_kib"] <= 0):
            raise ValueError(f"Insufficient real scroll and full-frame coverage: {path}")
        if (mode == "adaptive2" and fields["switches"] < 8) or (
            mode == "fixed2" and fields["switches"] != 0
        ):
            raise ValueError(f"Invalid production adaptive/rollback route witness: {path}")
        if len(fields.get("digest", "")) != 16:
            raise ValueError(f"Missing deterministic full-frame checksum: {path}")
        rows[label] = fields
    if set(rows) != set(CASES):
        raise ValueError(f"Missing or extra scene rows in {path}: {list(rows)}")
    return rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("folder", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    rows = {}
    for trial in TRIALS:
        for mode in MODES:
            path = args.folder / f"scroll-pipeline-{mode}-trial-{trial}.log"
            for case, fields in parse(path, mode).items():
                rows[(trial, mode, case)] = fields

    # Both policies must have seen identical native visual output and workload
    # geometry; otherwise a timing comparison is invalid rather than slower.
    for case in CASES:
        fingerprints = {
            (r["digest"], r["frames"], r["shifts"], r["full_frames"],
             r["small_frames"])
            for trial in TRIALS for mode in MODES
            for r in [rows[(trial, mode, case)]]
        }
        if len(fingerprints) != 1:
            raise ValueError(f"Different frame outputs or workloads in {case}: {fingerprints}")

    def m(case: str, mode: str, metric: str) -> float:
        return median(rows[(trial, mode, case)][metric] for trial in TRIALS)

    def gain(fixed: float, routed: float) -> float:
        return (fixed - routed) * 100.0 / fixed

    lines = [
        "# Akula: pointer event to CPU framebuffer A/B",
        "",
        "Linux release test profile; three separate processes per mode,",
        "alternating order on the same runner. Both modes invoke the real",
        "Renderer::new(2) constructor; 0 explicitly disables the production",
        "adaptive route and 1 enables it. Identical pixels, natural scroll",
        "geometry, full-invalidation coverage and serial-route selection",
        "are verified before any latency comparison is reported.",
        "",
        "**Measurement scope:** synchronous pointer/animation event, Masonry",
        "layout/scene generation and CPU draw_frame. **Not** GPU/window",
        "presentation, compositor, vsync, display input latency or app FPS.",
        "Timings are workload/runner-specific; no numeric performance gate.",
        "",
        "| Scene | Mode | Total sum (ms) | Total p50 / p95 (us) | Event sum (ms) | Scene sum (ms) | Draw sum (ms) | Peak RSS (MiB) |",
        "|---|---|---:|---:|---:|---:|---:|---:|",
    ]
    for case in CASES:
        for mode in MODES:
            lines.append(
                f"| {case} | {mode} | {m(case,mode,'total_sum_us')/1000:.3f} | "
                f"{m(case,mode,'total_p50_us'):.1f} / {m(case,mode,'total_p95_us'):.1f} | "
                f"{m(case,mode,'input_sum_us')/1000:.3f} | "
                f"{m(case,mode,'scene_sum_us')/1000:.3f} | "
                f"{m(case,mode,'draw_sum_us')/1000:.3f} | "
                f"{m(case,mode,'peak_kib')/1024:.2f} |"
            )
        fixed_total = m(case, "fixed2", "total_sum_us")
        adaptive_total = m(case, "adaptive2", "total_sum_us")
        fixed_draw = m(case, "fixed2", "draw_sum_us")
        adaptive_draw = m(case, "adaptive2", "draw_sum_us")
        fixed_scene = m(case, "fixed2", "scene_sum_us")
        adaptive_scene = m(case, "adaptive2", "scene_sum_us")
        lines.extend([
            "",
            f"**{case}:** input-to-CPU-frame sampled total "
            f"**{gain(fixed_total, adaptive_total):+.2f}%** improvement; "
            f"draw stage **{gain(fixed_draw, adaptive_draw):+.2f}%**, "
            f"scene stage **{gain(fixed_scene, adaptive_scene):+.2f}%**.",
            "",
        ])
    lines.extend([
        "A positive number means lower adaptive elapsed time; a negative",
        "number means an adaptive regression. Stage medians are independent,",
        "not additive. The run is a diagnostic benchmark, not an FPS claim.",
        "",
        "Validation requires 12 scene rows from 6 independent processes,",
        "including full-frame parity <=1/255 at six selected frames in each",
        "scene, the actual serial switching witness and matching frame digest.",
        "",
    ])
    report = "\n".join(lines)
    args.output.write_text(report, encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
