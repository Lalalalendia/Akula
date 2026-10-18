#!/usr/bin/env python3
"""Fail-closed quota check across *separate sender processes* over real loopback QUIC.

Only localhost; not an Internet, NAT, phone, or wire-byte billing experiment.
"""
import argparse
import pathlib
import re
import select
import subprocess
import tempfile


def run(binary, *args, ok=True):
    p = subprocess.run([str(binary), *map(str, args)], capture_output=True, text=True, timeout=25)
    if ok and p.returncode != 0:
        raise AssertionError(f"command failed: {p.args}\nstdout:{p.stdout}\nstderr:{p.stderr}")
    if not ok and p.returncode == 0:
        raise AssertionError(f"command unexpectedly succeeded: {p.args}\n{p.stdout}")
    return p


def start(binary, server_key, client_id, book, quota_dir, limit):
    command = [str(binary), "serve", "--key", str(server_key), "--book", str(book),
               "--allow", client_id, "--bind", "127.0.0.1:0",
               "--global-quota", str(limit), "--peer-quota", str(limit),
               "--quota-state", str(quota_dir)]
    p = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                         text=True, bufsize=1)
    ready, _, _ = select.select([p.stdout], [], [], 30)
    if not ready:
        terminate(p)
        raise AssertionError("sender did not become ready")
    line = p.stdout.readline().strip()
    match = re.fullmatch(r"READY id=([a-z0-9]+) addr=([^ ]+) hash=([0-9a-f]{64})", line)
    if not match:
        error = p.stderr.read() if p.poll() is not None else "(sender still running)"
        terminate(p)
        raise AssertionError(f"bad sender line: {line} {error}")
    return p, match[1], match[2]


def terminate(p):
    if p.poll() is None:
        p.terminate()
    try:
        p.communicate(timeout=5)
    except subprocess.TimeoutExpired:
        p.kill()
        p.communicate(timeout=5)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--binary", required=True)
    args = ap.parse_args()
    binary = pathlib.Path(args.binary).resolve()

    with tempfile.TemporaryDirectory(prefix="akula-quota-process-restart-") as temp:
        root = pathlib.Path(temp)
        sender_key = root / "sender.key"
        reader_key = root / "reader.key"
        sender_id = run(binary, "identity", "--key", sender_key).stdout.strip().split()[-1]
        client_id = run(binary, "identity", "--key", reader_key).stdout.strip().split()[-1]
        data = bytes((i * 23) % 256 for i in range(600_001))
        book = root / "book.bin"
        book.write_bytes(data)
        ledger = root / "quota-state"
        limit = 262_144  # One verified chunk, exactly; re-request must be denied.

        server, observed_id, addr = start(binary, sender_key, client_id, book, ledger, limit)
        assert observed_id == sender_id
        try:
            readargs = ("fetch", "--key", reader_key, "--remote-id", sender_id,
                        "--remote-addr", addr, "--stage", root / "stage",
                        "--output", root / "copy.bin", "--max-parts", "1")
            first = run(binary, *readargs)
            assert "PARTIAL verified=1 total=3 new=1" in first.stdout, first.stdout
            assert not (root / "copy.bin").exists()

            # Different sender process must not acquire same ledger while first owns it.
            second = run(binary, "serve", "--key", sender_key, "--book", book,
                         "--allow", client_id, "--quota-state", ledger,
                         "--global-quota", str(limit), "--peer-quota", str(limit), ok=False)
            assert "locked" in second.stderr.lower(), second.stderr
        finally:
            terminate(server)

        server, new_id, new_addr = start(binary, sender_key, client_id, book, ledger, limit)
        assert new_id == sender_id
        try:
            denied = run(binary, "fetch", "--key", reader_key, "--remote-id", sender_id,
                         "--remote-addr", new_addr, "--stage", root / "stage",
                         "--output", root / "copy.bin", "--max-parts", "1", ok=False)
            assert "status 2" in denied.stderr.lower(), denied.stderr
            assert (root / "stage" / "part-0000.bin").stat().st_size == limit
            assert not (root / "stage" / "part-0001.bin").exists()
            assert not (root / "copy.bin").exists()
        finally:
            terminate(server)

        # Corrupt persisted journal: starting the server must fail, never reset counters.
        journal = ledger / "quota.journal"
        corrupted = bytearray(journal.read_bytes())
        corrupted[-1] ^= 1
        journal.write_bytes(corrupted)
        blocked = run(binary, "serve", "--key", sender_key, "--book", book,
                      "--allow", client_id, "--quota-state", ledger,
                      "--global-quota", str(limit), "--peer-quota", str(limit), ok=False)
        assert "hash mismatch" in blocked.stderr.lower(), blocked.stderr

    print("PASS: persisted quota after sender restart; concurrent sender refused; "
          "tampered journal refused; no partial output")


if __name__ == "__main__":
    main()
