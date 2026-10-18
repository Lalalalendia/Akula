#!/usr/bin/env python3
"""Measure the sender's own CPU/RSS/FDs under unauthorized QUIC connections.

Laboratory ONLY: 24 separate client processes with 127/8 loopback source IPs.
The load is not sent to public services, and the host is NOT a WAN/NAT proxy.
Requires Linux /proc; each measurement is for the sender PID, not the clients.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import os
import pathlib
import re
import select
import subprocess
import sys
import tempfile
import threading
import time


UNKNOWN = 24
BYTES = 65_536
SOURCE_PREFIX = "127.0.0."
TIMEOUT = 22


def call(binary: pathlib.Path, *args: object, timeout: float = TIMEOUT) -> subprocess.CompletedProcess:
    return subprocess.run(
        [str(binary), *(str(arg) for arg in args)],
        capture_output=True, text=True, timeout=timeout, check=False,
    )


def identity(binary: pathlib.Path, key: pathlib.Path) -> str:
    p = call(binary, "identity", "--key", key)
    assert p.returncode == 0 and p.stdout.startswith("IDENTITY "), (p.stdout, p.stderr)
    return p.stdout.strip().split()[-1]


def snapshot(pid: int) -> dict[str, int]:
    """Linux /proc stats: fields 14/15 = utime/stime, parsed after '(comm)'."""
    stat = pathlib.Path(f"/proc/{pid}/stat").read_text()
    _, suffix = stat.rsplit(")", 1)
    fields = suffix.split()
    cpu_ticks = int(fields[11]) + int(fields[12])
    status = pathlib.Path(f"/proc/{pid}/status").read_text()
    def read_kib(label: str) -> int:
        row = next(line for line in status.splitlines() if line.startswith(label))
        return int(row.split()[1])
    return {
        "cpu_ticks": cpu_ticks,
        "rss_kib": read_kib("VmRSS:"),
        "hwm_kib": read_kib("VmHWM:"),
        "threads": int(next(line.split()[1] for line in status.splitlines() if line.startswith("Threads:"))),
        "fds": len(list(pathlib.Path(f"/proc/{pid}/fd").iterdir())),
    }


def stop_process(p: subprocess.Popen) -> None:
    if p.poll() is None:
        p.terminate()
    try:
        p.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        p.kill()
        p.communicate(timeout=5)


def fetch(binary: pathlib.Path, key: pathlib.Path, server_id: str,
          addr: str, root: pathlib.Path, label: str, bind: str = "127.0.0.1:0"):
    return call(
        binary, "fetch", "--key", key, "--remote-id", server_id,
        "--remote-addr", addr, "--bind", bind,
        "--stage", root / f"stage-{label}", "--output", root / f"output-{label}",
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    args = parser.parse_args()
    assert sys.platform.startswith("linux"), "Linux /proc required"
    binary = pathlib.Path(args.binary).resolve()
    assert binary.is_file(), binary
    hz = os.sysconf("SC_CLK_TCK")
    assert hz > 0

    with tempfile.TemporaryDirectory(prefix="akula-isolated-sender-") as tmp:
        root = pathlib.Path(tmp)
        source_file = root / "book.bin"
        original = bytes(range(256)) * (BYTES // 256)
        source_file.write_bytes(original)
        server_key, good_key = root / "server.key", root / "good.key"
        server_id = identity(binary, server_key)
        known_id = identity(binary, good_key)
        stranger_keys = []
        for i in range(UNKNOWN):
            key = root / f"stranger-{i}.key"
            identity(binary, key)
            stranger_keys.append(key)

        journal = root / "ledger"
        server = subprocess.Popen(
            [str(binary), "serve", "--key", str(server_key),
             "--book", str(source_file), "--allow", known_id,
             "--bind", "127.0.0.1:0", "--quota-state", str(journal),
             "--global-quota", str(2 * BYTES), "--peer-quota", str(2 * BYTES),
             "--global-rate", "4194304", "--peer-rate", "2097152"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1,
        )

        try:
            assert server.stdout is not None
            ready, _, _ = select.select([server.stdout], [], [], 30)
            assert ready, "sender startup timed out"
            line = server.stdout.readline().strip()
            match = re.fullmatch(r"READY id=([a-z0-9]+) addr=([^ ]+) hash=([0-9a-f]{64})", line)
            assert match is not None and match[1] == server_id, line
            server_addr = match[2]

            baseline_start = time.monotonic()
            baseline = fetch(binary, good_key, server_id, server_addr, root, "baseline")
            baseline_ms = round((time.monotonic() - baseline_start) * 1000)
            assert baseline.returncode == 0, (baseline.stdout, baseline.stderr)
            assert (root / "output-baseline").read_bytes() == original
            assert server.poll() is None, "sender exited unexpectedly"

            idle = snapshot(server.pid)
            measurements = [idle]
            done = threading.Event()
            sampler_error = []
            def sample_process():
                while not done.is_set():
                    try:
                        measurements.append(snapshot(server.pid))
                    except (FileNotFoundError, ProcessLookupError) as error:
                        sampler_error.append(str(error))
                        return
                    time.sleep(0.025)

            sampler = threading.Thread(target=sample_process, daemon=True)
            start_gate = threading.Barrier(UNKNOWN + 1)
            def stranger_request(i: int):
                start_gate.wait(timeout=20)
                started = time.monotonic()
                result = fetch(
                    binary, stranger_keys[i], server_id, server_addr,
                    root, f"stranger-{i}", f"{SOURCE_PREFIX}{i + 2}:0",
                )
                return (result.returncode, round((time.monotonic() - started) * 1000), i)

            sampler.start()
            stress_start = time.monotonic()
            with concurrent.futures.ThreadPoolExecutor(max_workers=UNKNOWN) as pool:
                futures = [pool.submit(stranger_request, i) for i in range(UNKNOWN)]
                start_gate.wait(timeout=20)
                time.sleep(0.06)
                good_start = time.monotonic()
                known = fetch(binary, good_key, server_id, server_addr, root, "under-load")
                good_ms = round((time.monotonic() - good_start) * 1000)
                stranger_results = [future.result(timeout=TIMEOUT + 8) for future in futures]

            elapsed_ms = round((time.monotonic() - stress_start) * 1000)
            done.set()
            sampler.join(timeout=2)
            assert not sampler_error, sampler_error
            measurements.append(snapshot(server.pid))
            assert server.poll() is None, "sender died under unknown-key load"
            assert known.returncode == 0, (known.stdout, known.stderr)
            assert (root / "output-under-load").read_bytes() == original
            assert all(rc != 0 for rc, _, _ in stranger_results), stranger_results
            assert all(not (root / f"output-stranger-{i}").exists() for i in range(UNKNOWN))

            # Baseline plus one allowed request, exactly two 64-KiB payload
            # reservations. An untrusted connection cannot spend this quota.
            # Header: 4+32+8+8+32 bytes; record: 8+32+8+32 bytes.
            assert (journal / "quota.journal").stat().st_size == 84 + 2 * 80
            denial_times = sorted(elapsed for _, elapsed, _ in stranger_results)
            p95_denial_ms = denial_times[22]  # nearest rank ceil(0.95 * 24) = 23
            cpu_ticks = measurements[-1]["cpu_ticks"] - idle["cpu_ticks"]
            assert cpu_ticks >= 0
            rss_peak = max(x["rss_kib"] for x in measurements)
            fd_peak = max(x["fds"] for x in measurements)
            threads_peak = max(x["threads"] for x in measurements)
            assert rss_peak >= idle["rss_kib"]
            assert fd_peak >= idle["fds"]

            print(
                "SENDER_ISOLATED_24 unknown_denied=24 known=ok "
                f"baseline_ms={baseline_ms} under_load_ms={good_ms} "
                f"denial_p95_ms={p95_denial_ms} elapsed_ms={elapsed_ms} "
                f"sender_cpu_ticks={cpu_ticks} sender_cpu_ms={cpu_ticks * 1000 / hz:.1f} "
                f"sender_rss_base_kib={idle['rss_kib']} "
                f"sender_rss_peak_kib={rss_peak} "
                f"sender_rss_growth_kib={rss_peak - idle['rss_kib']} "
                f"sender_fds_base={idle['fds']} sender_fds_peak={fd_peak} "
                f"sender_threads_peak={threads_peak} samples={len(measurements)} "
                "scope=127/8_linux_loopback"
            )
        finally:
            stop_process(server)


if __name__ == "__main__":
    main()
