#!/usr/bin/env python3
"""Real two-process localhost smoke test. NOT two computers/NAT or a relay test."""
import argparse
import hashlib
import os
import pathlib
import re
import select
import subprocess
import tempfile
import time


def cmd(binary, *args, ok=True):
    result = subprocess.run([str(binary), *map(str, args)], capture_output=True, text=True, timeout=25)
    if ok and result.returncode != 0:
        raise AssertionError(f"failed: {result.args}\n{result.stdout}\n{result.stderr}")
    if not ok and result.returncode == 0:
        raise AssertionError(f"unexpected success: {result.args}\n{result.stdout}")
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    opts = ap.parse_args()
    binary = pathlib.Path(opts.binary).resolve()
    with tempfile.TemporaryDirectory(prefix="akula-iroh-two-process-") as base:
        base = pathlib.Path(base)
        server_key = base / "server.key"
        client_key = base / "client.key"
        stranger_key = base / "stranger.key"
        server_id = cmd(binary, "identity", "--key", server_key).stdout.strip().split()[-1]
        client_id = cmd(binary, "identity", "--key", client_key).stdout.strip().split()[-1]
        cmd(binary, "identity", "--key", stranger_key)
        book = base / "book.txt"
        book_bytes = bytes((i * 91) % 256 for i in range(620_001))
        book.write_bytes(book_bytes)
        serve = subprocess.Popen([str(binary), "serve", "--key", str(server_key),
            "--book", str(book), "--allow", client_id, "--bind", "127.0.0.1:0",
            "--global-quota", "1000000", "--peer-quota", "1000000",
            "--quota-state", str(base / "quota-state")],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        try:
            assert serve.stdout
            ready, _, _ = select.select([serve.stdout], [], [], 30)
            if not ready:
                raise AssertionError("server did not become ready")
            line = serve.stdout.readline().strip()
            match = re.fullmatch(r"READY id=([a-z0-9]+) addr=([^ ]+) hash=([0-9a-f]{64})", line)
            if not match or match[1] != server_id:
                raise AssertionError(f"unexpected ready line: {line}")
            addr = match[2]
            stage = base / "stage"
            out = base / "copy.txt"
            args = ("fetch", "--key", client_key, "--remote-id", server_id,
                "--remote-addr", addr, "--stage", stage, "--output", out)
            initial = cmd(binary, *args, "--max-parts", "1")
            assert "PARTIAL verified=1 total=3 new=1" in initial.stdout, initial.stdout
            assert not out.exists(), "incomplete book leaked into output"
            assert stage.joinpath("part-0000.bin").is_file()
            final = cmd(binary, *args)
            assert "COMPLETE bytes=620001 chunks=3 new=2" in final.stdout, final.stdout
            metric = re.search(r"CLIENT_WIRE_METRICS udp_tx_bytes=(\d+) udp_rx_bytes=(\d+) udp_tx_datagrams=(\d+) udp_rx_datagrams=(\d+) connections=(\d+) payload_verified=(\d+) udp_rx_to_payload_ratio=([0-9.]+)", final.stdout)
            assert metric, final.stdout
            tx_bytes, rx_bytes, tx_packets, rx_packets, connections, payload, ratio = metric.groups()
            assert min(int(tx_bytes), int(rx_bytes), int(tx_packets), int(rx_packets)) > 0
            assert int(connections) == 3, final.stdout  # One manifest plus two missing chunks
            assert int(payload) == len(book_bytes)
            assert float(ratio) > 0
            assert out.read_bytes() == book_bytes
            assert hashlib.sha256(out.read_bytes()).digest() == hashlib.sha256(book_bytes).digest()
            bad = cmd(binary, "fetch", "--key", stranger_key, "--remote-id", server_id,
                "--remote-addr", addr, "--stage", base / "badstage", "--output", base / "badbook", ok=False)
            assert not (base / "badbook").exists()
            assert "error" in bad.stderr.lower() or bad.returncode != 0
            print("PASS real separate processes; resumable persisted chunks; no partial output; untrusted peer denied")
        finally:
            serve.terminate()
            try:
                serve.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                serve.kill()
                serve.communicate(timeout=5)


if __name__ == "__main__":
    main()