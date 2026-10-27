#!/usr/bin/env python3
"""M3a: PID-scoped native Windows large Markdown open and wheel-memory pilot.

Diagnostic only: SendInput delivery is not proof of visual scroll or UIA
interactivity. Live input runs only on disposable GitHub-hosted Windows VMs.
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
import tempfile
import time
from statistics import median

if os.name == "nt":
    from native_windows_memory_pilot import close_owned_windows, isolated_env, sample
    from native_mipmap_window_ab import windows_gui_witness

SIZES = (30000, 100000)
WHEELS = [(12, -720), (16, -720), (20, -720), (24, -720),
          (28, 720), (32, 720), (36, 720), (40, 720)]
MIB = 1048576


def checksum(path):
    h = hashlib.sha256()
    with path.open("rb") as f:
        for part in iter(lambda: f.read(MIB), b""):
            h.update(part)
    return h.hexdigest()


def fixture(path, count):
    if count not in (*SIZES, 32) or path.exists() or (
        path.parent.exists() and any(path.parent.iterdir())
    ):
        raise RuntimeError("Refuse unsafe or duplicate synthetic fixture")
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("wb") as f:
        for i in range(count):
            if i == 0:
                line = f"# M3-{count} native Akula document"
            elif i % 997 == 0:
                line = f"## Section {i} — Кириллица"
            elif i % 17 == 0:
                line = f"- {i:06d} **bold** and *italic* [[Test link]]"
            else:
                line = f"Paragraph {i:06d}: native windowed Markdown. Привет Akula."
            f.write((line + "\n").encode("utf-8"))
    if path.read_bytes().count(b"\n") != count:
        raise RuntimeError("Incorrect line count")
    return {"lines": count, "bytes": path.stat().st_size, "sha256": checksum(path)}


def verify(path, manifest):
    if not path.is_file() or path.stat().st_size != manifest["bytes"] or checksum(path) != manifest["sha256"]:
        raise RuntimeError("Synthetic note content was changed")


class Wheel:
    """Windows OS wheel events only after foreground editor-title/PID checks."""
    def __init__(self):
        if os.name != "nt":
            raise RuntimeError("Windows only")
        self.win = ctypes.WinDLL("user32", use_last_error=True)
        class Rect(ctypes.Structure):
            _fields_ = [("left", wintypes.LONG), ("top", wintypes.LONG),
                        ("right", wintypes.LONG), ("bottom", wintypes.LONG)]
        class Mouse(ctypes.Structure):
            _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG),
                        ("mouseData", wintypes.DWORD), ("dwFlags", wintypes.DWORD),
                        ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_size_t)]
        class Payload(ctypes.Union):
            _fields_ = [("mouse", Mouse)]
        class Input(ctypes.Structure):
            _fields_ = [("type", wintypes.DWORD), ("payload", Payload)]
        self.Rect, self.Mouse, self.Payload, self.Input = Rect, Mouse, Payload, Input
        self.callback = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        w = self.win
        w.EnumWindows.argtypes = [self.callback, wintypes.LPARAM]
        w.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        w.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        w.GetWindowRect.argtypes = [wintypes.HWND, ctypes.POINTER(Rect)]
        w.IsWindowVisible.argtypes = [wintypes.HWND]
        w.IsIconic.argtypes = [wintypes.HWND]
        w.SetForegroundWindow.argtypes = [wintypes.HWND]
        w.GetForegroundWindow.restype = wintypes.HWND
        w.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
        w.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(Input), ctypes.c_int]
        w.SendInput.restype = wintypes.UINT

    def editor(self, pid, stem):
        found = []
        @self.callback
        def scan(hwnd, _):
            owner = wintypes.DWORD()
            self.win.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
            if owner.value != pid or not self.win.IsWindowVisible(hwnd) or self.win.IsIconic(hwnd):
                return True
            title = ctypes.create_unicode_buffer(512)
            self.win.GetWindowTextW(hwnd, title, 512)
            if not (title.value.startswith(stem) and "Akula" in title.value):
                return True
            rect = self.Rect()
            if self.win.GetWindowRect(hwnd, ctypes.byref(rect)) and (
                rect.right - rect.left >= 400 and rect.bottom - rect.top >= 300
            ):
                found.append((hwnd, rect))
                return False
            return True
        self.win.EnumWindows(scan, 0)
        return found[0] if found else None

    def inject(self, pid, stem, delta):
        if delta not in (-720, 720) or not windows_gui_witness(pid):
            raise RuntimeError("Native PID/window witness lost or unsafe delta")
        target = self.editor(pid, stem)
        if target is None:
            raise RuntimeError("Editor title is no longer the requested test document")
        hwnd, box = target
        self.win.SetForegroundWindow(hwnd)
        if self.win.GetForegroundWindow() != hwnd:
            raise RuntimeError("Refusing wheel event on background app")
        x, y = (box.left + box.right) // 2, box.top + 2 * (box.bottom - box.top) // 3
        if not self.win.SetCursorPos(x, y):
            raise RuntimeError("Unable to target test editor body")
        data = self.Input(type=0, payload=self.Payload(
            mouse=self.Mouse(dx=0, dy=0, mouseData=delta & 0xffffffff,
                             dwFlags=0x0800, time=0, dwExtraInfo=0)))
        if self.win.SendInput(1, ctypes.byref(data), ctypes.sizeof(self.Input)) != 1:
            raise RuntimeError("SendInput wheel event not accepted by Windows")


def run(binary, out, note, note_data):
    count = note_data["lines"]
    folder = out / "runs" / str(count)
    if folder.exists():
        raise RuntimeError("Duplicate run")
    folder.mkdir(parents=True)
    profile = folder / "profile"
    profile.mkdir()
    command = [str(binary), "--vault", str(note.parent), "--data-dir", str(profile),
               "--open", str(note), "--focus", "--size", "1000x650", "--threads", "2"]
    rows, events = [], []
    editor_at, visible_at, process_ticks = None, None, None
    graceful, forced = False, False
    win = Wheel()
    process = None
    begin = time.monotonic()
    try:
        with (folder / "process.log").open("wb") as log:
            process = subprocess.Popen(command, env=isolated_env(),
                                       stdout=log, stderr=subprocess.STDOUT)
            while True:
                now = time.monotonic()
                elapsed = now - begin
                if process.poll() is not None:
                    raise RuntimeError("Akula exited while opening/scrolling large note")
                memory = sample(process.pid)
                if process_ticks is None:
                    process_ticks = memory["created_ticks"]
                elif memory["created_ticks"] != process_ticks:
                    raise RuntimeError("PID creation ticks unexpectedly changed")
                if visible_at is None and windows_gui_witness(process.pid):
                    visible_at = elapsed
                if editor_at is None and win.editor(process.pid, note.stem) and windows_gui_witness(process.pid):
                    editor_at = elapsed
                if editor_at is None and elapsed > 45:
                    raise RuntimeError("Requested native editor title absent after 45s")
                relative = elapsed - editor_at if editor_at is not None else -1
                if editor_at is not None and len(events) < len(WHEELS) and relative >= WHEELS[len(events)][0]:
                    delta = WHEELS[len(events)][1]
                    win.inject(process.pid, note.stem, delta)
                    events.append({"relative_s": round(relative, 3), "delta": delta})
                phase = ("startup" if editor_at is None else
                         "before" if relative < 12 else "after" if relative >= 40 else "wheel")
                rows.append({"elapsed_s": round(elapsed, 3), "relative_s": round(relative, 3),
                             "phase": phase, "pid": process.pid, **memory})
                if editor_at is not None and relative >= 60:
                    break
                time.sleep(max(0, 0.25 - (time.monotonic() - now)))
            if len(events) != len(WHEELS) or not win.editor(process.pid, note.stem):
                raise RuntimeError("Wheel count or final document title mismatch")
            if not close_owned_windows(process.pid):
                raise RuntimeError("No PID-owned window to close")
            try:
                process.wait(timeout=20)
                graceful = process.returncode == 0
            except subprocess.TimeoutExpired:
                pass
            if not graceful:
                raise RuntimeError("Nonzero or timed-out process exit")
    finally:
        if process is not None and process.poll() is None:
            process.kill()
            process.wait(timeout=15)
            forced = True
        if rows:
            with (folder / "samples.csv").open("w", newline="", encoding="utf-8") as f:
                writer = csv.DictWriter(f, fieldnames=list(rows[0]))
                writer.writeheader()
                writer.writerows(rows)
        (folder / "witness.json").write_text(json.dumps({
            "size": count, "editor_title_proxy_s": editor_at, "uncloaked_hwnd_proxy_s": visible_at,
            "events": events, "graceful": graceful, "forced": forced,
            "os_input_does_not_prove_visual_scrolling": True
        }, indent=2), encoding="utf-8")
    verify(note, note_data)
    before = [r for r in rows if r["phase"] == "before" and r["relative_s"] >= 6]
    after = [r for r in rows if r["phase"] == "after" and r["relative_s"] >= 48]
    if len(before) < 8 or len(after) < 8:
        raise RuntimeError("Insufficient pre/post wheel memory samples")
    mid = lambda data: round(median(r["private_bytes"] for r in data) / MIB, 3)
    return {"lines": count, "bytes": note_data["bytes"], "editor_title_proxy_s": round(editor_at, 3),
            "wheel_events": len(events), "samples": len(rows), "private_before_mib": mid(before),
            "private_after_mib": mid(after), "sampled_peak_private_mib": round(max(
                r["private_bytes"] for r in rows) / MIB, 3),
            "os_peak_working_set_mib": round(max(r["peak_working_set_bytes"] for r in rows) / MIB, 3),
            "graceful": graceful}


def selftest():
    with tempfile.TemporaryDirectory() as temp:
        path = Path(temp) / "vault" / "M3-000032.md"
        data = fixture(path, 32)
        verify(path, data)
        with path.open("ab") as f:
            f.write(b"modified")
        try:
            verify(path, data)
        except RuntimeError:
            pass
        else:
            raise AssertionError("Modified fixture should fail")
    print("M3A_FIXTURE_SELFTEST_PASS", flush=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--binary", type=Path)
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()
    if args.self_test:
        selftest()
        return
    if (os.name != "nt" or os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted"
            or os.environ.get("RUNNER_OS") != "Windows"):
        raise RuntimeError("OS pointer input allowed only on disposable GitHub-hosted Windows")
    if args.binary is None or not args.binary.is_file() or args.out is None:
        raise RuntimeError("Expected --binary and --out")
    if args.out.exists() and any(args.out.iterdir()):
        raise RuntimeError("Refuse nonempty output path")
    args.out.mkdir(parents=True, exist_ok=True)
    manifest = {"source_sha": os.environ.get("AKULA_PROBE_SOURCE_HEAD", "unknown"),
                "exe_sha256": checksum(args.binary), "mode": "unpackaged Windows Release installed feature"}
    (args.out / "environment.json").write_text(json.dumps(manifest, indent=2), encoding="utf-8")
    results = []
    for n in SIZES:
        path = args.out / "fixtures" / str(n) / f"M3-{n:06d}.md"
        data = fixture(path, n)
        print(f"M3A_START lines={n} bytes={data['bytes']}", flush=True)
        result = run(args.binary.resolve(), args.out.resolve(), path.resolve(), data)
        results.append(result)
        (args.out / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
        print(f"M3A_PASS lines={n} wheels={result['wheel_events']} private={result['private_after_mib']}", flush=True)
    lines = ["# M3a: Windows native Markdown open + OS wheel delivery", "",
             "Diagnostic: no actual visual scrolling, first-interactive UIA, installed NSIS or Undo acceptance.",
             f"Source {manifest['source_sha']}; EXE SHA-256 {manifest['exe_sha256']}.", "",
             "| Lines | Bytes | Document HWND proxy s | Private before wheels MiB | Private after wheels MiB | Sampled Private peak MiB | OS peak WS MiB | Wheel inputs |",
             "|---:|---:|---:|---:|---:|---:|---:|---:|"]
    for r in results:
        lines.append(f"| {r['lines']} | {r['bytes']} | {r['editor_title_proxy_s']} | {r['private_before_mib']} | {r['private_after_mib']} | {r['sampled_peak_private_mib']} | {r['os_peak_working_set_mib']} | {r['wheel_events']} |")
    lines.extend(["", "Next: prove actual scroll offset/paint and separately perform real edit+Undo+readback."])
    text = "\n".join(lines) + "\n"
    (args.out / "summary.md").write_text(text, encoding="utf-8")
    print(text)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"M3A_FAIL {error!r}", file=sys.stderr, flush=True)
        raise
