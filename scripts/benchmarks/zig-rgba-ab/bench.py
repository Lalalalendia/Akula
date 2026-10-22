#!/usr/bin/env python3
"""Single-runner Rust/Zig RGBA kernel A/B. No application or CI integration."""
import argparse
import ctypes
import hashlib
import json
import os
import random
import resource
import statistics
import time
from pathlib import Path

ARMS = ("rust_scalar", "zig_scalar", "rust_packed", "zig_packed")
SMALL = ((0, 0), (0, 33), (33, 0), (1, 1), (1, 2), (2, 1),
         (1, 33), (33, 1), (3, 3), (3, 5), (5, 3), (17, 19), (128, 127))
LARGE = ((1280, 884), (1281, 885), (4096, 2160))

def fixture(w, h):
    rng = random.Random((w << 17) ^ (h << 3) ^ 0xA51ACAFE)
    src = bytearray(rng.randbytes(w * h * 4))
    # Produce valid premultiplied RGBA, including transparent and opaque texels.
    for at in range(0, len(src), 4):
        a = src[at + 3]
        src[at] = min(src[at], a)
        src[at + 1] = min(src[at + 1], a)
        src[at + 2] = min(src[at + 2], a)
    return src

def reference(src, w, h):
    if not w or not h or (w == 1 and h == 1):
        return (0, b"")
    dw, dh = (w + 1) // 2, (h + 1) // 2
    out = bytearray(dw * dh * 4)
    alpha = False
    for y in range(dh):
        y0, y1 = 2 * y, min(2 * y + 1, h - 1)
        for x in range(dw):
            x0, x1 = 2 * x, min(2 * x + 1, w - 1)
            a = (y0 * w + x0) * 4
            b = (y0 * w + x1) * 4
            c = (y1 * w + x0) * 4
            d = (y1 * w + x1) * 4
            p = (y * dw + x) * 4
            for channel in range(4):
                value = (src[a + channel] + src[b + channel] +
                         src[c + channel] + src[d + channel] + 2) // 4
                out[p + channel] = value
                if channel == 3 and value != 255:
                    alpha = True
    return (2 if alpha else 1, bytes(out))

def bind(path, symbol):
    lib = ctypes.CDLL(str(path))
    fn = getattr(lib, symbol)
    fn.argtypes = (ctypes.POINTER(ctypes.c_uint8),
                   ctypes.POINTER(ctypes.c_uint8), ctypes.c_size_t, ctypes.c_size_t)
    fn.restype = ctypes.c_size_t
    return (lib, fn)

def invoke(fn, src_buffer, dst_buffer, w, h):
    return fn(src_buffer, dst_buffer, w, h)

def prepare(src, w, h):
    inp = (ctypes.c_uint8 * max(1, len(src))).from_buffer_copy(src if src else b"\0")
    n = ((w + 1) // 2) * ((h + 1) // 2) * 4 if w and h and (w, h) != (1, 1) else 0
    out = (ctypes.c_uint8 * max(1, n))()
    return inp, out, n

def measure(libs, report_path):
    fns = {
        key: bind(libs["zig" if key.startswith("zig") else "rust"],
                  "rgba_half_packed" if key.endswith("packed") else "rgba_half_scalar")
        for key in ARMS
    }
    report = {"environment": {"sha": os.getenv("GITHUB_SHA", "local"),
               "machine": os.uname().machine, "cpu_count": os.cpu_count()},
              "strict_parity": [], "measurements": []}
    for w, h in SMALL + LARGE:
        src = fixture(w, h)
        inp, out, n = prepare(src, w, h)
        # Independent oracle for small dimensions; Rust scalar for full-size
        # comparisons after the small oracle has proven the formula.
        expected_flag, expected = (
            reference(src, w, h) if (w, h) in SMALL else (None, None)
        )
        for key in ARMS:
            keep, fn = fns[key]  # keep CDLL alive
            got_flag = invoke(fn, inp, out, w, h)
            got = bytes(out[:n])
            if expected is None:
                _, baseline = fns["rust_scalar"]
                baseline_out = (ctypes.c_uint8 * max(1, n))()
                expected_flag = invoke(baseline, inp, baseline_out, w, h)
                expected = bytes(baseline_out[:n])
            if got_flag != expected_flag or got != expected:
                raise AssertionError(
                    f"parity FAIL {key} {w}x{h}: flag={got_flag}, "
                    f"expected={expected_flag}, first_mismatch="
                    f"{next((i for i, (a,b) in enumerate(zip(got, expected)) if a != b), -1)}")
        report["strict_parity"].append({"size": [w, h], "sha256": hashlib.sha256(expected).hexdigest(),
                                       "metadata_flag": expected_flag, "all_four_equal": True})
        print(f"PARITY {w}x{h}: byte exact for all four variants", flush=True)
        if (w, h) not in LARGE:
            continue
        timings = {name: [] for name in ARMS}
        # One runner, same src + dst, reverse order on alternate repetitions.
        for turn in range(12):
            order = ARMS if turn % 2 == 0 else ARMS[::-1]
            for key in order:
                keep, fn = fns[key]
                t0 = time.perf_counter_ns()
                flag = invoke(fn, inp, out, w, h)
                elapsed_ms = (time.perf_counter_ns() - t0) / 1_000_000
                assert flag == expected_flag
                if turn >= 2:
                    timings[key].append(elapsed_ms)
        for key, values in timings.items():
            sorted_values = sorted(values)
            report["measurements"].append({
                "size": [w, h], "variant": key,
                "median_ms": round(statistics.median(values), 4),
                "p95_ms": round(sorted_values[9] * 0.55 + sorted_values[8] * 0.45, 4),
                "samples": len(values), "sha256": hashlib.sha256(expected).hexdigest()
            })
            print(f"BENCH {w}x{h} {key}: median={statistics.median(values):.4f} ms", flush=True)
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print("WROTE", report_path)

def memory_one(libs, key):
    w, h = 4096, 2160
    src = fixture(w, h)
    inp, out, n = prepare(src, w, h)
    lib, fn = bind(libs["zig" if key.startswith("zig") else "rust"],
                   "rgba_half_packed" if key.endswith("packed") else "rgba_half_scalar")
    for _ in range(3):
        result = invoke(fn, inp, out, w, h)
        assert result == 2
    print(json.dumps({"variant": key, "max_rss_kib_with_python_fixture":
        resource.getrusage(resource.RUSAGE_SELF).ru_maxrss, "output_bytes": n}), flush=True)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--rust", type=Path, required=True)
    parser.add_argument("--zig", type=Path, required=True)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--rss-one", choices=ARMS)
    args = parser.parse_args()
    libs = {"rust": args.rust.resolve(), "zig": args.zig.resolve()}
    if args.rss_one:
        memory_one(libs, args.rss_one)
    else:
        if args.report is None:
            parser.error("--report is required in benchmark mode")
        measure(libs, args.report)
if __name__ == "__main__":
    main()
