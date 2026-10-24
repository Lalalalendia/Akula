#!/usr/bin/env python3
"""Isolated, reproducible language comparison. No product code is imported."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import sys
import time
import platform
import math

ROOT = Path(__file__).resolve().parent
EXPECTED = ("664579", "1936361925")
REPETITIONS = 5

def definitions(bin_dir):
    b = str(bin_dir)
    # All compiled modes optimize for execution, not binary size. Defaults
    # and runtimes differ; version, linker, flags and environment are recorded.
    return [
        dict(name="C", tool="gcc", version=["gcc", "--version"], source="src/bench.c", build=["gcc", "-std=c11", "-O3", "-march=x86-64", "src/bench.c", "-o", f"{b}/c"], exe=f"{b}/c"),
        dict(name="C++", tool="g++", version=["g++", "--version"], source="src/bench.cpp", build=["g++", "-std=c++17", "-O3", "-march=x86-64", "src/bench.cpp", "-o", f"{b}/cpp"], exe=f"{b}/cpp"),
        dict(name="Rust", tool="rustc", version=["rustc", "--version"], source="src/bench.rs", build=["rustc", "-C", "opt-level=3", "-C", "target-cpu=x86-64", "src/bench.rs", "-o", f"{b}/rust"], exe=f"{b}/rust"),
        dict(name="Zig", tool="zig", version=["zig", "version"], source="src/bench.zig", build=["zig", "build-exe", "src/bench.zig", "-O", "ReleaseFast", f"-femit-bin={b}/zig"], exe=f"{b}/zig"),
        dict(name="Odin", tool="odin", version=["odin", "version"], source="src/bench.odin", build=["odin", "build", "src/bench.odin", "-file", "-o:speed", f"-out:{b}/odin"], exe=f"{b}/odin"),
        dict(name="C3", tool="c3c", version=["c3c", "--version"], source="src/bench.c3", build=["c3c", "compile", "src/bench.c3", "-O3", "-o", f"{b}/c3"], exe=f"{b}/c3"),
        dict(name="D (GDC)", tool="gdc", version=["gdc", "--version"], source="src/bench.d", build=["gdc", "-O3", "src/bench.d", "-o", f"{b}/d"], exe=f"{b}/d"),
        dict(name="Nim", tool="nim", version=["nim", "--version"], source="src/bench.nim", build=["nim", "c", "-d:release", "--opt:speed", "--hints:off", f"--out:{b}/nim", "src/bench.nim"], exe=f"{b}/nim"),
        dict(name="Go", tool="go", version=["go", "version"], source="src/bench.go", build=["go", "build", "-o", f"{b}/go", "src/bench.go"], exe=f"{b}/go"),
        dict(name="Fortran", tool="gfortran", version=["gfortran", "--version"], source="src/bench.f90", build=["gfortran", "-O3", "src/bench.f90", "-o", f"{b}/fortran"], exe=f"{b}/fortran"),
        dict(name="Free Pascal", tool="fpc", version=["fpc", "-iV"], source="src/bench.pas", build=["fpc", "-O3", "-Xs", f"-o{b}/pascal", "src/bench.pas"], exe=f"{b}/pascal"),
        dict(name="Python (control)", tool="python3", version=["python3", "--version"], source="src/bench.py", build=None, exe=None),
    ]

def capture(cmd, timeout=120):
    return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, timeout=timeout, env={**os.environ, "LC_ALL": "C"})

def once(command, timelog, timeout=100):
    cmd = ["/usr/bin/time", "-f", "%e\t%U\t%M", "-o", str(timelog), "--"] + command
    t = capture(cmd, timeout=timeout)
    # Zig writes its result to stderr. Ignore nonnumeric diagnostics.
    observed = tuple(line.strip() for line in (t.stdout + "\n" + t.stderr).splitlines() if line.strip().isdigit())
    return t, observed

def run(args):
    output = Path(args.output).resolve()
    builddir = output / "bin"
    logdir = output / "logs"
    builddir.mkdir(parents=True, exist_ok=True)
    logdir.mkdir(parents=True, exist_ok=True)
    items = definitions(builddir)
    records = {}
    for spec in items:
        name = spec["name"]
        src = ROOT / spec["source"]
        row = dict(language=name, status="not_run", version=None, source_sha256=hashlib.sha256(src.read_bytes()).hexdigest(), build_command=spec["build"], build_s=None, binary_bytes=None, median_s=None, p95_s=None, max_rss_kib=None, samples_s=[], error=None)
        records[name] = row
        if not shutil.which(spec["tool"]):
            row["status"] = "toolchain_missing"
            row["error"] = f"Compiler {spec['tool']} unavailable; no measurement."
            continue
        try:
            ver = capture(spec["version"], timeout=30)
            row["version"] = (ver.stdout + "\n" + ver.stderr).strip().splitlines()[:2]
            if spec["build"]:
                start = time.perf_counter()
                completed = capture(spec["build"], timeout=240)
                row["build_s"] = round(time.perf_counter() - start, 3)
                (logdir / (name.lower().replace(" ", "_").replace("(", "").replace(")", "") + "-build.log")).write_text(completed.stdout + "\n" + completed.stderr)
                if completed.returncode != 0:
                    row["status"] = "compile_error"
                    row["error"] = (completed.stdout + completed.stderr)[-1800:]
                    continue
                exe = Path(spec["exe"])
                if not exe.is_file() and name == "C3":
                    matches = [p for p in (builddir / "c3").rglob("*") if p.is_file() and os.access(p, os.X_OK)]
                    if len(matches) == 1: exe = matches[0]
                if not exe.is_file():
                    row["status"] = "compile_error"
                    row["error"] = "Compiler returned success without producing the expected executable."
                    continue
                row["binary_bytes"] = exe.stat().st_size
                command = [str(exe)]
            else:
                command = [sys.executable, "-B", spec["source"]]
            t, observed = once(command, logdir / "preflight.time")
            if t.returncode != 0 or observed != EXPECTED:
                row["status"] = "wrong_output" if t.returncode == 0 else "runtime_error"
                row["error"] = f"Exit {t.returncode}, expected {EXPECTED}, observed {observed}; stderr: {t.stderr[-400:]}"
                continue
            row["status"] = "ready"
            row["command"] = command
        except (OSError, subprocess.TimeoutExpired) as err:
            row["status"] = "compile_error"
            row["error"] = repr(err)

    ready = [x for x in items if records[x["name"]]["status"] == "ready"]
    rng = random.Random(20261025)
    # Interleaved on a single runner, to reduce ordering bias.
    for round_id in range(args.repetitions):
        shuffled = list(ready)
        rng.shuffle(shuffled)
        for spec in shuffled:
            row = records[spec["name"]]
            if row["status"] != "ready": continue
            log = logdir / f"{spec['name'].replace(' ', '_')}-{round_id}.time"
            try:
                start_measure = time.perf_counter()
                outcome, observed = once(row["command"], log)
                elapsed = time.perf_counter() - start_measure
                if outcome.returncode != 0 or observed != EXPECTED:
                    row["status"] = "wrong_output" if outcome.returncode == 0 else "runtime_error"
                    row["error"] = f"Repeat {round_id}: code {outcome.returncode}, got {observed}"
                    continue
                wall, user, rss = log.read_text().strip().split("\t")
                row["samples_s"].append(round(elapsed, 6))
                row.setdefault("cpu_samples_s", []).append(float(user))
                row.setdefault("rss_samples_kib", []).append(int(rss))
            except (OSError, ValueError, subprocess.TimeoutExpired) as err:
                row["status"] = "runtime_error"
                row["error"] = repr(err)
    for row in records.values():
        if row["status"] == "ready" and len(row["samples_s"]) == args.repetitions:
            row["status"] = "ok"
            row["median_s"] = round(statistics.median(row["samples_s"]), 5)
            vals = sorted(row["samples_s"])
            row["p95_s"] = vals[math.ceil(0.95 * len(vals)) - 1]
            row["max_rss_kib"] = max(row["rss_samples_kib"])
        elif row["status"] == "ready":
            row["status"] = "incomplete"
        row.pop("command", None)
    cpu_model = "unknown"
    try:
        cpu_model = next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name"))
    except (OSError, StopIteration): pass
    env = dict(run_date=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), utc="UTC", os=platform.platform(), cpu=cpu_model, architecture=platform.machine(), github_sha=os.environ.get("GITHUB_SHA"), github_ref=os.environ.get("GITHUB_REF"), repetitions=args.repetitions, sieve_limit=10_000_000, hash_iterations=20_000_000, expected_stdout=list(EXPECTED), note="Wall clock includes process startup and both operations; Python is not a low-level candidate. Source programs are single threaded; runtimes may use threads.")
    report = dict(environment=env, languages=list(records.values()))
    (output / "results.json").write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n")
    cols = ["language", "status", "version", "build_s", "binary_bytes", "median_s", "p95_s", "max_rss_kib", "error"]
    with (output / "results.csv").open("w", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=cols, extrasaction="ignore")
        writer.writeheader()
        for row in records.values():
            writer.writerow({**row, "version": ";".join(row["version"] or [])})
    lines = ["# Language comparison (experimental)", "", "Same 10M-element sieve + 20M-iteration unsigned 32-bit rolling hash; output parity required; all numbers below are from *this* GitHub runner only.", "", f"Machine: {cpu_model}; {platform.platform()}; SHA {env['github_sha'] or 'local'}.", "", "| Language | Status | Median, s | p95, s | Peak RSS, KiB | Binary, bytes | Compile, s |", "|---|---|---:|---:|---:|---:|---:|"]
    for row in records.values():
        vals = [row["language"], row["status"], row["median_s"], row["p95_s"], row["max_rss_kib"], row["binary_bytes"], row["build_s"]]
        lines.append("| " + " | ".join("—" if v is None else str(v) for v in vals) + " |")
    lines += ["", "Caveats: not a universal language-speed ranking; algorithms, bounds checks, GC/ARC and runtime differ; C3/Odin/Zig use release builds; Python is an interpreter control. Build time includes linker, not toolchain downloads. RSS is whole-process peak; p95 is nearest-rank among five measurements and noisy. Runner CPU is shared. Missing compilers and incorrect outputs are not comparable. Full versions, flags, sample lists, errors and hashes: results.json.", ""]
    (output / "summary.md").write_text("\n".join(lines))
    print("\n".join(lines))
    for row in records.values():
        if row["status"] != "ok":
            print("DIAGNOSTIC", row["language"], row["status"], row["error"])
    baseline = {"C", "C++", "Go", "Python (control)"}
    failures = [n for n in baseline if records[n]["status"] != "ok"]
    if failures:
        print(f"Core baseline failures: {failures}", file=sys.stderr)
        return 1
    return 0

if __name__ == "__main__":
    p = argparse.ArgumentParser()
    p.add_argument("--output", default=".language-compare")
    p.add_argument("--repetitions", type=int, default=REPETITIONS)
    args = p.parse_args()
    if not 2 <= args.repetitions <= 10:
        p.error("2..10 repetitions")
    sys.exit(run(args))
