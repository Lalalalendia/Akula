#!/usr/bin/env python3
"""Byte-accurate parity and paired timed rounds, one independent process."""
import argparse
import ctypes
import hashlib
import json
import os
import statistics
import time
from pathlib import Path
from bench import SMALL, LARGE, fixture, reference, prepare, bind

ARMS = (
    ("rust_generic_scalar", "rust_generic", "rgba_half_scalar"),
    ("zig_generic_scalar", "zig_generic", "rgba_half_scalar"),
    ("rust_generic_packed", "rust_generic", "rgba_half_packed"),
    ("zig_generic_packed", "zig_generic", "rgba_half_packed"),
    ("rust_generic_unaligned", "rust_generic", "rgba_half_unaligned"),
    ("rust_generic_row", "rust_generic", "rgba_half_row"),
    ("rust_native_packed", "rust_native", "rgba_half_packed"),
    ("rust_native_unaligned", "rust_native", "rgba_half_unaligned"),
    ("rust_native_row", "rust_native", "rgba_half_row"),
    ("zig_native_packed", "zig_native", "rgba_half_packed"),
)
MEASURE = ((1280, 884), (4096, 2160))

def run(args):
    libs = dict((key, Path(value).resolve()) for key, value in (
        ("rust_generic", args.rust_generic), ("rust_native", args.rust_native),
        ("zig_generic", args.zig_generic), ("zig_native", args.zig_native)))
    fns = {key: bind(libs[lib], symbol) for key, lib, symbol in ARMS}
    checks, records = [], []
    for w, h in SMALL + LARGE:
        for mode in ("mixed", "opaque", "zero"):
            if (w, h) in LARGE and mode == "zero":
                continue
            src = fixture(w, h)
            if mode == "opaque":
                for pos in range(3, len(src), 4):
                    src[pos] = 255
            elif mode == "zero":
                src[:] = b"\x00" * len(src)
            inp, dst, out_len = prepare(src, w, h)
            if (w, h) in SMALL:
                expected_flag, expected = reference(src, w, h)
            else:
                baseline = (ctypes.c_uint8 * max(out_len, 1))()
                expected_flag = fns["rust_generic_scalar"][1](inp, baseline, w, h)
                expected = bytes(baseline[:out_len])
            for name, _, _ in ARMS:
                actual_flag = fns[name][1](inp, dst, w, h)
                got = bytes(dst[:out_len])
                if actual_flag != expected_flag or got != expected:
                    mismatch = next((i for i, (x, y) in enumerate(zip(got, expected))
                                     if x != y), -1)
                    raise AssertionError(f"PARITY FAILED: {name}, {w}x{h}, {mode}, byte={mismatch}, "
                                         f"flag={actual_flag} expected={expected_flag}")
            checks.append({"size": [w, h], "mode": mode, "parity": True,
                           "opaque_flag": expected_flag,
                           "sha256": hashlib.sha256(expected).hexdigest()})
            if (w, h) not in MEASURE or mode != "mixed":
                continue
            timings = {key: [] for key, _, _ in ARMS}
            names = [key for key, _, _ in ARMS]
            for turn in range(12):
                shift = (turn + 3 * args.repetition) % len(names)
                order = names[shift:] + names[:shift]
                if turn % 2:
                    order.reverse()
                for key in order:
                    t0 = time.perf_counter_ns()
                    flag = fns[key][1](inp, dst, w, h)
                    elapsed = (time.perf_counter_ns() - t0) * 1e-6
                    if flag != expected_flag:
                        raise AssertionError(f"opacity changed {key}")
                    if turn >= 2:
                        timings[key].append(elapsed)
            for name, vals in timings.items():
                ordered = sorted(vals)
                records.append({"size": [w, h], "variant": name,
                                "median_ms": statistics.median(vals),
                                "p95_ms": ordered[9],
                                "samples": len(vals)})
                print(f"RUN {args.repetition} {w}x{h} {name}: "
                      f"{statistics.median(vals):.4f} ms", flush=True)
    assert len(checks) == 45, len(checks)  # 13 small*3 + 3 large*2
    args.output.write_text(json.dumps({
        "sha": os.getenv("GITHUB_SHA", "local"), "run": args.repetition,
        "checks": checks, "measurements": records}, indent=2))
    print(f"PARITY PASS: {len(checks)} cases * {len(ARMS)} implementations", flush=True)

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    for key in ("rust_generic", "rust_native", "zig_generic", "zig_native"):
        p.add_argument("--" + key.replace("_", "-"), required=True)
    p.add_argument("--repetition", type=int, required=True)
    p.add_argument("--output", type=Path, required=True)
    run(p.parse_args())
