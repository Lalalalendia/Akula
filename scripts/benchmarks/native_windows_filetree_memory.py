#!/usr/bin/env python3
"""AKULA PERF B2/B3: isolated flat/nested 1k/10k/50k Win32 GUI memory pilot.

No production code changes, no user vaults, no claims of physical first-interactive
or full installed NSIS behavior. The SQLite counter is a completion proxy only.
"""
from __future__ import annotations
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
from statistics import median

from native_windows_memory_pilot import (
    close_owned_windows, hash_file, isolated_env, sample, windows_gui_witness,
)

SIZES = (1000, 10000, 50000)
MODES = ("flat", "nested_collapsed", "nested_expanded")
SECONDS = {1000: 40, 10000: 80, 50000: 160}
INTERVAL_S = 0.2
INDEX_POLL_S = 2.0
MIB = 1048576


def note(index: int) -> bytes:
    return (
        f"# Synthetic Akula file {index:05d}\n\n"
        "Fixture for native file-tree and search-index profiling only. "
        "UTF-8/кириллица.\n"
    ).encode("utf-8")


def folder_for(index: int) -> str:
    return f"d_{index // 250:04d}"


def make_corpus(root: Path, count: int, shape: str) -> dict:
    if count < 1 or shape not in ("flat", "nested") or root.exists():
        raise ValueError("Invalid or existing corpus; refusing overwrite")
    root.mkdir(parents=True)
    files = []
    folders = {}
    for i in range(count):
        directory = root if shape == "flat" else root / folder_for(i)
        if directory != root:
            if directory.name not in folders:
                directory.mkdir()
                folders[directory.name] = 0
            folders[directory.name] += 1
        file = directory / f"n_{i:05d}.md"
        file.write_bytes(note(i))
        if i in (0, count // 2, count - 1):
            files.append({"path": str(file.relative_to(root)).replace("\\", "/"),
                          "sha256": hashlib.sha256(note(i)).hexdigest()})
    observed = sum(1 for path in root.rglob("*.md") if path.is_file())
    if observed != count:
        raise RuntimeError(f"Fixture count mismatch: {observed}/{count}")
    return {"root": str(root.resolve()), "size": count, "shape": shape,
            "folder_count": len(folders), "sentinels": files}


def verify_corpus(corpus: dict) -> None:
    root = Path(corpus["root"])
    for s in corpus["sentinels"]:
        file = root / s["path"]
        if hash_file(file) != s["sha256"]:
            raise RuntimeError("Synthetic source modified: " + str(file))
    count = sum(1 for path in root.rglob("*.md") if path.is_file())
    if count != corpus["size"]:
        raise RuntimeError("Number of test notes changed")


def comparable(path: Path) -> str:
    return str(path.resolve()).replace("\\", "/").lower()


def seed_expanded(profile: Path, corpus: dict, mode: str) -> int:
    if profile.exists():
        raise RuntimeError("Refusing to reuse test profile in cold run")
    profile.mkdir(parents=True)
    root = Path(corpus["root"])
    expanded = sorted(p.name for p in root.iterdir() if p.is_dir()) if mode == "nested_expanded" else []
    if mode == "nested_expanded" and len(expanded) != corpus["folder_count"]:
        raise RuntimeError("Expanded directory inventory mismatch")
    store = {"workspaces": {comparable(root): expanded}, "sorts": {}, "colors": {}}
    (profile / "expanded-folders.json").write_text(json.dumps(store, ensure_ascii=False), encoding="utf-8")
    return len(expanded)


def index_documents(profile: Path) -> int | None:
    files = list((profile / "search-v2").glob("*/documents.sqlite3"))
    if len(files) != 1:
        return None
    try:
        # Non-authoritative progress witness; read-only, nonblocking and
        # excluded from indexer's correctness or search latency claims.
        uri = files[0].resolve().as_uri() + "?mode=ro"
        with sqlite3.connect(uri, uri=True, timeout=0.1) as db:
            return int(db.execute("SELECT COUNT(*) FROM documents").fetchone()[0])
    except (sqlite3.Error, OSError):
        return None


def run_process(binary: Path, root: Path, manifest: dict, mode: str,
                phase: str, duration: int, interval: float) -> dict:
    count = manifest["size"]
    label = f"{count}-{mode}-{phase}"
    path = root / "runs" / label
    if path.exists():
        raise RuntimeError("Run already exists: " + str(path))
    path.mkdir(parents=True)
    profile = root / "profiles" / f"{count}-{mode}"
    expanded_count = seed_expanded(profile, manifest, mode) if phase == "cold" else None
    if phase == "warm" and not profile.is_dir():
        raise RuntimeError("Warm run requires preceding cold profile")

    args = [str(binary), "--vault", manifest["root"], "--data-dir", str(profile),
            "--size", "1000x650", "--threads", "2"]
    rows = []
    visible_at = None
    window = None
    ready_at = None
    latest_indexed = None
    created_ticks = None
    next_index_poll = 0.0
    graceful = False
    forced = False
    start = time.monotonic()
    process = None
    try:
        with (path / "process.log").open("wb") as log:
            process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, env=isolated_env())
            while True:
                now = time.monotonic()
                elapsed = now - start
                if elapsed >= duration:
                    break
                if process.poll() is not None:
                    raise RuntimeError(f"Akula stopped early exit={process.returncode} {label}")
                info = sample(process.pid)
                if created_ticks is None:
                    created_ticks = info["created_ticks"]
                elif created_ticks != info["created_ticks"]:
                    raise RuntimeError("PID reused during measurement")
                if visible_at is None:
                    window = windows_gui_witness(process.pid)
                    if window:
                        visible_at = round(elapsed, 3)
                if elapsed >= next_index_poll:
                    latest_indexed = index_documents(profile)
                    next_index_poll = elapsed + INDEX_POLL_S
                    if latest_indexed == count and ready_at is None:
                        ready_at = round(elapsed, 3)
                rows.append({"elapsed_s": round(elapsed, 3), "pid": process.pid,
                             "gui_visible": int(visible_at is not None),
                             "indexed_docs": -1 if latest_indexed is None else latest_indexed,
                             **info})
                time.sleep(max(0, interval - (time.monotonic() - now)))
            final_indexed = index_documents(profile)
            if visible_at is None or visible_at > duration - 10:
                raise RuntimeError(f"No timely uncloaked visible native Akula window {label}")
            if final_indexed != count or ready_at is None:
                raise RuntimeError(f"Index metadata incomplete {label} {final_indexed}/{count}; not a warm baseline")
            if ready_at > duration - 10:
                raise RuntimeError(f"Index too late for a settled checkpoint {label} @ {ready_at}s")
            if len(rows) < (duration / interval) * 0.9:
                raise RuntimeError(f"Insufficient process samples {label}: {len(rows)}")
            if not close_owned_windows(process.pid):
                raise RuntimeError("No owned HWND for graceful exit")
            try:
                process.wait(timeout=20)
                graceful = process.returncode == 0
            except subprocess.TimeoutExpired:
                graceful = False
            if not graceful:
                raise RuntimeError(f"Unclean shutdown after B2/B3 {label}")
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=15)
            forced = True
        if rows:
            with (path / "samples.csv").open("w", newline="", encoding="utf-8") as out:
                writer = csv.DictWriter(out, fieldnames=list(rows[0]))
                writer.writeheader()
                writer.writerows(rows)
        (path / "witness.json").write_text(json.dumps({
            "label": label, "visible_proxy_s": visible_at,
            "index_metadata_count": index_documents(profile), "index_metadata_complete_s": ready_at,
            "expected_documents": count, "sample_count": len(rows),
            "expanded_seeded": expanded_count,
            "window": window, "graceful": graceful, "forced": forced,
        }, indent=2), encoding="utf-8")

    verify_corpus(manifest)
    return {
        "size": count, "mode": mode, "phase": phase,
        "visible_proxy_s": visible_at, "indexed_documents": count,
        "index_metadata_complete_s": ready_at, "samples": len(rows),
        "private_end_mib": round(rows[-1]["private_bytes"] / MIB, 2),
        "private_sample_peak_mib": round(max(x["private_bytes"] for x in rows) / MIB, 2),
        "peak_working_set_mib": round(max(x["peak_working_set_bytes"] for x in rows) / MIB, 2),
        "cpu_ms_end": rows[-1]["cpu_ms"],
        "graceful": graceful,
    }


def summary(data: list[dict], environment: dict) -> str:
    lines = [
        "# Akula B2/B3 — synthetic Windows file-tree memory and startup pilot", "",
        "DIAGNOSTIC: unpackaged Windows Release with installed feature, no user vault.",
        "Visible uncloaked HWND is a startup proxy, not a proven first usable frame.",
        "SQLite documents count is a metadata completion proxy, not search correctness.",
        "Cold/warm here means fresh/reused app data profile, **not** cleared OS file caches.",
        "No full 600-second M2 idle, physical installation, DXGI attribution or production RAM budget.",
        "", f"Source: {environment['source_sha']}; binary SHA-256: {environment['exe_sha256']}",
        f"Win32 desktop: {environment['desktop_px']}; sample: {environment['interval_ms']} ms",
        "",
        "| Notes | Tree | Profile | HWND proxy s | Metadata complete s | Final Private MiB | Sampled peak Private MiB | Peak WS MiB |",
        "|---:|---|---|---:|---:|---:|---:|---:|",
    ]
    for row in data:
        lines.append("| {size} | {mode} | {phase} | {visible_proxy_s:.2f} | {index_metadata_complete_s:.2f} | {private_end_mib:.2f} | {private_sample_peak_mib:.2f} | {peak_working_set_mib:.2f} |".format(**row))
    lines += ["", "No performance improvement claim until repeated same-runner A/B, independent product correctness and first-interactive proof.", ""]
    return "\n".join(lines)


def self_test() -> None:
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        for shape in ("flat", "nested"):
            c = make_corpus(root / shape, 5, shape)
            assert c["size"] == 5
            assert len(c["sentinels"]) == 3
            verify_corpus(c)
            seed = root / f"profile-{shape}"
            n = seed_expanded(seed, c, "nested_expanded" if shape == "nested" else "flat")
            assert n == int(shape == "nested")
            stored = json.loads((seed / "expanded-folders.json").read_text(encoding="utf-8"))
            assert list(stored["workspaces"]) == [comparable(Path(c["root"]))]
    print("B2_B3_FIXTURE_SELFTEST_PASS")


def main() -> None:
    arg = argparse.ArgumentParser(description=__doc__)
    arg.add_argument("--self-test", action="store_true")
    arg.add_argument("--binary", type=Path)
    arg.add_argument("--out", type=Path)
    args = arg.parse_args()
    if args.self_test:
        self_test()
        return
    if os.name != "nt":
        raise RuntimeError("Real Win32 GUI runner is required")
    if args.binary is None or not args.binary.is_file() or args.out is None:
        raise ValueError("--binary and --out are required")
    if args.out.exists() and any(args.out.iterdir()):
        raise RuntimeError("Refuse to use a nonempty output directory")
    args.out.mkdir(parents=True, exist_ok=True)
    import ctypes
    ctypes.windll.user32.GetSystemMetrics.argtypes = [ctypes.c_int]
    ctypes.windll.user32.GetSystemMetrics.restype = ctypes.c_int
    screen = [ctypes.windll.user32.GetSystemMetrics(i) for i in (0, 1)]
    if screen[0] < 1000 or screen[1] < 720:
        raise RuntimeError(f"Win32 interactive desktop unavailable: {screen}")
    environment = {
        "source_sha": os.environ.get("AKULA_PROBE_SOURCE_HEAD", "unknown"),
        "exe_sha256": hash_file(args.binary),
        "desktop_px": screen, "interval_ms": int(INTERVAL_S * 1000),
        "count": SIZES, "modes": MODES, "max_duration_by_size_s": SECONDS,
    }
    (args.out / "environment.json").write_text(json.dumps(environment, indent=2), encoding="utf-8")
    corpus = {}
    for count in SIZES:
        for shape in ("flat", "nested"):
            print(f"B2_B3_CORPUS_START count={count} shape={shape}", flush=True)
            corpus[(count, shape)] = make_corpus(args.out / "fixtures" / str(count) / shape, count, shape)
            print(f"B2_B3_CORPUS_PASS count={count} shape={shape}", flush=True)
    data = []
    for count in SIZES:
        for mode in MODES:
            fixture = corpus[(count, "flat" if mode == "flat" else "nested")]
            for phase in ("cold", "warm"):
                print(f"B2_B3_START n={count} mode={mode} phase={phase}", flush=True)
                result = run_process(args.binary.resolve(), args.out.resolve(), fixture, mode,
                                     phase, SECONDS[count], INTERVAL_S)
                data.append(result)
                (args.out / "results.json").write_text(json.dumps(data, indent=2), encoding="utf-8")
                print(f"B2_B3_PASS n={count} mode={mode} phase={phase} visible={result['visible_proxy_s']} indexed={result['index_metadata_complete_s']} private={result['private_end_mib']}", flush=True)
    result = summary(data, environment)
    (args.out / "summary.md").write_text(result, encoding="utf-8")
    print(result, flush=True)


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"B2_B3_FAIL {exc!r}", file=sys.stderr, flush=True)
        raise
