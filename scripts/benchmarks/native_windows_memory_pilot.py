#!/usr/bin/env python3
"""Read-only M1/B0-B1 Win32 memory pilot, synthetic profiles only.

Release binary uses --features installed but is not NSIS-installed or a physical
end-user host. An uncloaked HWND is not proof of UI Automation interactivity.
"""
from __future__ import annotations
import argparse
import csv
import ctypes
from ctypes import wintypes
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from statistics import median

if os.name == "nt":
    from native_mipmap_window_ab import windows_gui_witness

    class MemoryCountersEx(ctypes.Structure):
        _fields_ = [
            ("cb", wintypes.DWORD),
            ("PageFaultCount", wintypes.DWORD),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
            ("PrivateUsage", ctypes.c_size_t),
        ]
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    psapi = ctypes.WinDLL("psapi", use_last_error=True)
    open_process = kernel.OpenProcess
    open_process.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    open_process.restype = wintypes.HANDLE
    close_handle = kernel.CloseHandle
    close_handle.argtypes = [wintypes.HANDLE]
    close_handle.restype = wintypes.BOOL
    get_memory = psapi.GetProcessMemoryInfo
    get_memory.argtypes = [wintypes.HANDLE, ctypes.POINTER(MemoryCountersEx), wintypes.DWORD]
    get_memory.restype = wintypes.BOOL
    get_times = kernel.GetProcessTimes
    get_times.argtypes = [wintypes.HANDLE] + [ctypes.POINTER(wintypes.FILETIME)] * 4
    get_times.restype = wintypes.BOOL
    get_handles = kernel.GetProcessHandleCount
    get_handles.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    get_handles.restype = wintypes.BOOL
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    enum_proc = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    enum_windows = user32.EnumWindows
    enum_windows.argtypes = [enum_proc, wintypes.LPARAM]
    enum_windows.restype = wintypes.BOOL
    window_pid = user32.GetWindowThreadProcessId
    window_pid.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    window_pid.restype = wintypes.DWORD
    post_message = user32.PostMessageW
    post_message.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    post_message.restype = wintypes.BOOL

def hash_file(path):
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1048576), b""):
            digest.update(chunk)
    return digest.hexdigest()

def sample(pid):
    if os.name != "nt":
        raise RuntimeError("Requires Win32")
    handle = open_process(0x0400 | 0x0010, False, pid)
    if not handle:
        raise RuntimeError(f"OpenProcess({pid}) failed: {ctypes.get_last_error()}")
    try:
        mem = MemoryCountersEx()
        mem.cb = ctypes.sizeof(mem)
        if not get_memory(handle, ctypes.byref(mem), mem.cb):
            raise RuntimeError(f"GetProcessMemoryInfo failed: {ctypes.get_last_error()}")
        created, exited, system, user = (wintypes.FILETIME() for _ in range(4))
        if not get_times(handle, ctypes.byref(created), ctypes.byref(exited),
                         ctypes.byref(system), ctypes.byref(user)):
            raise RuntimeError(f"GetProcessTimes failed: {ctypes.get_last_error()}")
        handles = wintypes.DWORD(0)
        if not get_handles(handle, ctypes.byref(handles)):
            raise RuntimeError("GetProcessHandleCount failed")
        ticks = lambda f: (f.dwHighDateTime << 32) | f.dwLowDateTime
        return {
            "created_ticks": ticks(created),
            "private_bytes": int(mem.PrivateUsage),
            "working_set_bytes": int(mem.WorkingSetSize),
            "peak_working_set_bytes": int(mem.PeakWorkingSetSize),
            "cpu_ms": (ticks(system) + ticks(user)) // 10000,
            "handles": int(handles.value),
        }
    finally:
        close_handle(handle)

def close_owned_windows(pid):
    sent = 0
    @enum_proc
    def find(hwnd, _):
        nonlocal sent
        found = wintypes.DWORD()
        window_pid(hwnd, ctypes.byref(found))
        if found.value == pid and post_message(hwnd, 0x0010, 0, 0):
            sent += 1
        return True
    enum_windows(find, 0)
    return sent

def isolated_env():
    env = os.environ.copy()
    for key in list(env):
        if key.startswith(("AKULA_EXPERIMENTAL_", "AQ_PROBE_", "AQ_TEST_", "AKULA_TEST_")) or key in ("AQ_TRACE", "AKULA_VELLO_SINGLE_RECT"):
            env.pop(key, None)
    return env

def run_one(binary, root, group, trial, seconds, interval_ms):
    location = root / f"trial-{trial}" / group
    if location.exists():
        raise RuntimeError(f"Refusing to overwrite existing trial: {location}")
    location.mkdir(parents=True)
    profile = location.parent / group.replace("warm", "cold") / "data" if group.endswith("warm") else location / "data"
    if group.endswith("warm"):
        if not profile.is_dir():
            raise RuntimeError("Warm profile has no preceding cold run")
    else:
        profile.mkdir()
    vault = location.parent / "empty_vault"
    args = [str(binary), "--data-dir", str(profile), "--size", "1000x650", "--threads", "2"]
    if group.startswith("empty_vault"):
        vault.mkdir(exist_ok=True)
        args += ["--vault", str(vault)]
    records = []
    visible_at = None
    window = None
    created_ticks = None
    process = None
    graceful = False
    forced = False
    start = time.monotonic()
    try:
        with (location / "process.log").open("wb") as log:
            process = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT, env=isolated_env())
            while time.monotonic() - start < seconds:
                if process.poll() is not None:
                    raise RuntimeError(f"Akula exited early, code={process.returncode}")
                now = time.monotonic()
                row = sample(process.pid)
                if created_ticks is None:
                    created_ticks = row["created_ticks"]
                elif created_ticks != row["created_ticks"]:
                    raise RuntimeError("PID reuse during sample")
                if visible_at is None:
                    window = windows_gui_witness(process.pid)
                    if window:
                        visible_at = round(now - start, 3)
                records.append({"elapsed_s": round(now - start, 3), "pid": process.pid,
                                "gui_visible": int(visible_at is not None), **row})
                time.sleep(max(0, interval_ms / 1000 - (time.monotonic() - now)))
            if visible_at is None or visible_at > seconds - 6:
                raise RuntimeError("No timely PID-owned visible uncloaked Win32 window")
            if not records or records[-1]["elapsed_s"] < seconds - 1:
                raise RuntimeError("Incomplete memory observations")
            if not close_owned_windows(process.pid):
                raise RuntimeError("No PID-owned HWND for graceful close")
            try:
                process.wait(timeout=12)
                graceful = process.returncode == 0
            except subprocess.TimeoutExpired:
                pass
            if not graceful:
                raise RuntimeError(f"Unclean app exit={process.poll()}")
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=15)
            forced = True
        if records:
            with (location / "samples.csv").open("w", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=list(records[0]))
                writer.writeheader()
                writer.writerows(records)
        (location / "witness.json").write_text(json.dumps({
            "trial": trial, "group": group, "window": window,
            "visible_hwnd_proxy_s": visible_at, "samples": len(records),
            "graceful_exit": graceful, "forced_kill": forced,
        }, indent=2), encoding="utf-8")
    return {
        "trial": trial, "group": group, "pid": process.pid,
        "window_visible_proxy_s": visible_at,
        "private_bytes_end": records[-1]["private_bytes"],
        "working_set_bytes_end": records[-1]["working_set_bytes"],
        "peak_private_sample_bytes": max(r["private_bytes"] for r in records),
        "peak_working_set_bytes": max(r["peak_working_set_bytes"] for r in records),
        "cpu_ms_end": records[-1]["cpu_ms"],
        "handles_end": records[-1]["handles"],
        "graceful_exit": graceful,
    }

GROUPS = (
    ("no_vault_cold", "No vault, fresh profile"),
    ("no_vault_warm", "No vault, reused profile"),
    ("empty_vault_cold", "Empty vault, fresh profile"),
    ("empty_vault_warm", "Empty vault, reused profile"),
)

def report(results, manifest):
    lines = [
        "# Akula M1/B0-B1 Win32 memory pilot", "",
        "**Diagnostic, not official installed Windows RAM baseline.**",
        "Release binary compiled with installed feature; NOT NSIS-installed.",
        "The first visible PID-owned HWND is NOT the first usable interaction.",
        "Only synthetic empty vaults; no personal files. No GPU/private-working-set attribution.", "",
        f"Source {manifest['git_sha']} ; EXE SHA-256 {manifest['exe_sha256']}.",
        f"Sampled every {manifest['interval_ms']} ms for {manifest['seconds']} seconds, {manifest['trials']} pairs.",
        "Private Bytes = private commit; sampled peak is not an OS peak counter.", "",
        "| Scenario | Runs | HWND proxy p50 (s) | Private Bytes end p50 (MiB) | Sampled private peak p50 (MiB) | Peak WS p50 (MiB) |",
        "|---|---:|---:|---:|---:|---:|",
    ]
    for group, label in GROUPS:
        found = [r for r in results if r["group"] == group and r["graceful_exit"]]
        if len(found) != manifest["trials"]:
            raise RuntimeError(f"Missing complete results for {group}")
        mid = lambda key: median(r[key] for r in found)
        mib = 1048576
        lines.append(f"| {label} | {len(found)} | {mid('window_visible_proxy_s'):.3f} | {mid('private_bytes_end') / mib:.1f} | {mid('peak_private_sample_bytes') / mib:.1f} | {mid('peak_working_set_bytes') / mib:.1f} |")
    lines += ["", "Next: 5 independent installed NSIS runs with UIA first-interactive witness;",
              "then M2 600s idle and B2/B3 flat-vs-nested 1k/10k/50k vaults.", ""]
    return "\n".join(lines)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--trials", type=int, default=3)
    parser.add_argument("--seconds", type=int, default=32)
    parser.add_argument("--interval-ms", type=int, default=200)
    options = parser.parse_args()
    if os.name != "nt":
        raise RuntimeError("Requires an interactive Windows runner")
    if not options.binary.is_file():
        raise RuntimeError("Release binary missing")
    if not 1 <= options.trials <= 5 or not 30 <= options.seconds <= 90 or not 100 <= options.interval_ms <= 1000:
        raise ValueError("Unbounded experiment is forbidden")
    if options.out.exists() and any(options.out.iterdir()):
        raise RuntimeError("Refuse to use nonempty output directory")
    options.out.mkdir(parents=True, exist_ok=True)
    user32.GetSystemMetrics.argtypes = [ctypes.c_int]
    user32.GetSystemMetrics.restype = ctypes.c_int
    desktop = (user32.GetSystemMetrics(0), user32.GetSystemMetrics(1))
    if desktop[0] < 1000 or desktop[1] < 720:
        raise RuntimeError(f"CI Windows interactive desktop too small: {desktop}")
    manifest = {
        "git_sha": os.environ.get("AKULA_PROBE_SOURCE_HEAD", os.environ.get("GITHUB_SHA", "unknown")),
        "exe_sha256": hash_file(options.binary), "binary": str(options.binary),
        "product_mode": "installed feature / unpackaged Windows Release",
        "desktop_px": desktop, "trials": options.trials, "seconds": options.seconds,
        "interval_ms": options.interval_ms,
    }
    (options.out / "environment.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    results = []
    for trial in range(1, options.trials + 1):
        for group, _ in GROUPS:
            print(f"AKULA_M1_START trial={trial} group={group}", flush=True)
            result = run_one(options.binary.resolve(), options.out.resolve(), group, trial,
                             options.seconds, options.interval_ms)
            results.append(result)
            print(f"AKULA_M1_PASS trial={trial} group={group} visible={result['window_visible_proxy_s']} private_end={result['private_bytes_end']}", flush=True)
    (options.out / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    summary = report(results, manifest)
    (options.out / "summary.md").write_text(summary, encoding="utf-8")
    print(summary, flush=True)

if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"AKULA_M1_FAIL {error!r}", file=sys.stderr)
        raise
