#!/usr/bin/env python3
"""Native Reader navigation/RSS stress with a *local* EPUB and real GUI frames.

For public CI use copyright-free synthetic fixtures only. The uploaded EPUB
must never be placed in GitHub Actions or uploaded as a job artifact.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
from pathlib import Path

ROW = re.compile(r"^READER_NAV_SAMPLE\s+(.*)$", re.MULTILINE)
FRAME = re.compile(r"^READER_GUI_FRAME\s+", re.MULTILINE)
NUMBER = re.compile(r"^-?\d+(?:\.\d+)?$")


def script_text(book: Path, count: int, pause: int, every: int) -> str:
    rows = [f"600 reader {book}", "0 mark paged_start"]

    def travel(mode: str, direction: str) -> None:
        for i in range(1, count + 1):
            rows.append(f"{pause} reader-{direction}")
            if i % every == 0 or i == count:
                rows.append(f"0 mark {mode}_{direction}_{i}")

    travel("paged", "next")
    travel("paged", "prev")
    rows.extend(["500 reader-mode scrolled", "0 mark scrolled_start"])
    travel("scrolled", "next")
    travel("scrolled", "prev")
    rows.extend([
        "500 close-reader",
        "0 mark after_close",
        f"600 reader {book}",
        "0 mark reopened",
        "500 close-reader",
        "0 mark final_closed",
    ])
    return "\n".join(rows) + "\n"


def parse_rows(text: str) -> list[dict]:
    result = []
    for match in ROW.finditer(text):
        values = {}
        for token in match.group(1).split():
            if "=" not in token:
                continue
            key, val = token.split("=", 1)
            if val in ("true", "false"):
                values[key] = val == "true"
            elif NUMBER.fullmatch(val):
                values[key] = float(val)
            else:
                values[key] = val
        result.append(values)
    return result


def execute(binary: Path, book: Path, out: Path, count: int, pause: int,
            every: int, timeout: int) -> dict:
    binary = binary.resolve(strict=True)
    book = book.resolve(strict=True)
    if not binary.is_file() or not book.is_file():
        raise ValueError("Missing binary or EPUB file")
    if book.suffix.lower() != ".epub":
        raise ValueError("Expected an EPUB")
    if not 1 <= count <= 800 or not 10 <= pause <= 500 or not 1 <= every <= count:
        raise ValueError("Invalid navigation count, pacing, or checkpoint frequency")
    out.mkdir(parents=True, exist_ok=True)
    for folder in ("vault", "data"):
        (out / folder).mkdir(exist_ok=True)
    (out / "vault" / "a.md").write_text("# Reader memory stress\n", encoding="utf-8")
    script = out / "probe-script.txt"
    script.write_text(script_text(book, count, pause, every), encoding="utf-8")
    probe, log, metrics = (out / name for name in ("memory.md", "app.log", "metrics.json"))
    env = dict(os.environ, AQ_PROBE_SCRIPT=str(script))
    cmd = [str(binary), "--vault", str(out / "vault"),
           "--data-dir", str(out / "data"), "--size", "1200x800",
           "--threads", "4", "--memory-probe", str(probe),
           "--metrics", str(metrics)]
    with log.open("wb") as dest:
        process = subprocess.Popen(cmd, stdout=dest, stderr=subprocess.STDOUT,
                                   env=env, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired as exc:
            try:
                state = Path(f"/proc/{process.pid}/status").read_text()
                (out / "timeout-process.txt").write_text(state, encoding="utf-8")
            except OSError:
                pass
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.wait(timeout=4)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.wait(timeout=3)
            raise TimeoutError(f"Reader process timed out after {timeout}s; "
                               f"last checkpoints in {probe}") from exc

    if code != 0:
        raise RuntimeError(f"Reader app failed with code {code}; see {log}")
    if not probe.exists() or not metrics.exists():
        raise AssertionError("Reader probe or frame metrics missing after process exit")
    text = probe.read_text(encoding="utf-8", errors="replace")
    rows = parse_rows(text)
    keys = [r.get("label") for r in rows]
    required = ["paged_start", "scrolled_start", "after_close", "reopened", "final_closed"]
    for k in required:
        if keys.count(k) != 1:
            raise AssertionError(f"Expected exactly one {k} sample, observed {keys}")
    checkpoints = len(range(every, count + 1, every)) + int(count % every != 0)
    if len(rows) != 5 + 4 * checkpoints:
        raise AssertionError(f"Missing samples: got {len(rows)}, expected {5+4*checkpoints}")
    for r in rows:
        for key in ["heap_current_mib", "heap_peak_mib", "rss_kib", "process_hwm_kib"]:
            if not isinstance(r.get(key), float) or r[key] <= 0:
                raise AssertionError(f"Invalid memory sample {key}: {r}")
    for label in ["paged_start", "scrolled_start", "reopened"]:
        item = next(r for r in rows if r["label"] == label)
        if not item.get("reader_active") or item.get("reader_layouts", 0) < 1:
            raise AssertionError(f"Reader not rendered at {label}: {item}")
    for label in ["after_close", "final_closed"]:
        if next(r for r in rows if r["label"] == label).get("reader_active"):
            raise AssertionError("Closed Reader retained its UI layer")
    for mode in ("paged", "scrolled"):
        direction = "reader_page" if mode == "paged" else "reader_scroll_px"
        start = next(r for r in rows if r["label"] == f"{mode}_start")
        forward = next(r for r in reversed(rows)
                       if str(r.get("label", "")).startswith(f"{mode}_next_"))
        backward = next(r for r in reversed(rows)
                        if str(r.get("label", "")).startswith(f"{mode}_prev_"))
        if not forward.get(direction, 0) > start.get(direction, 0):
            raise AssertionError(f"{mode} forward navigation did not advance: {forward}")
        if not backward.get(direction, 0) < forward.get(direction, 0):
            raise AssertionError(f"{mode} reverse navigation did not return: {backward}")
    measured = json.loads(metrics.read_text())
    if measured.get("frames", 0) < count * 2:
        raise AssertionError(f"Insufficient native GUI frames for stress: {measured}")
    if len(FRAME.findall(log.read_text(errors="replace"))) < 2:
        raise AssertionError("Reader first-text-present marker missing from reopen")
    summary = {
        "book_bytes": book.stat().st_size,
        "navigation_steps": count * 4,
        "frame_count": measured["frames"],
        "rss_at_paged_start_mib": round(next(r for r in rows if r["label"]=="paged_start")["rss_kib"]/1024, 2),
        "max_checkpoint_rss_mib": round(max(r["rss_kib"] for r in rows)/1024, 2),
        "rss_after_first_close_mib": round(next(r for r in rows if r["label"]=="after_close")["rss_kib"]/1024, 2),
        "rss_after_reopen_mib": round(next(r for r in rows if r["label"]=="reopened")["rss_kib"]/1024, 2),
        "rss_after_final_close_mib": round(next(r for r in rows if r["label"]=="final_closed")["rss_kib"]/1024, 2),
        "max_checkpoint_layouts": int(max(r.get("reader_layouts", 0) for r in rows)),
        "max_reported_heap_peak_mib": round(max(r["heap_peak_mib"] for r in rows), 2),
        "samples": rows,
        "scope": "real native Winit/Masonry window, memory-probe, not physical display scanout",
    }
    (out / "results.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2)+"\n")
    return summary


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--binary", required=True, type=Path)
    p.add_argument("--book", required=True, type=Path)
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--count", type=int, default=60)
    p.add_argument("--pause-ms", type=int, default=55)
    p.add_argument("--checkpoint-every", type=int, default=20)
    p.add_argument("--timeout-s", type=int, default=115)
    opts = p.parse_args()
    try:
        result = execute(opts.binary, opts.book, opts.out, opts.count,
                         opts.pause_ms, opts.checkpoint_every, opts.timeout_s)
    except Exception as exc:
        print(f"READER_NAV_MEMORY_FAILURE {type(exc).__name__}: {exc}", file=sys.stderr)
        return 1
    compact = {k: v for k, v in result.items() if k != "samples"}
    print("READER_NAV_MEMORY_RESULT " + json.dumps(compact, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
