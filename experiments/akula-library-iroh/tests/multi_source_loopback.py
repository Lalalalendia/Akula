#!/usr/bin/env python3
"""Distinct Linux loopback-source QUIC clients against isolated Akula laboratory.

Not a distributed WAN test. No public addresses receive packets.
"""
import argparse
import concurrent.futures
import pathlib
import re
import select
import subprocess
import tempfile
import time

def call(binary, *args, timeout=25):
    return subprocess.run([str(binary), *map(str, args)], capture_output=True,
                          text=True, timeout=timeout)

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    args = parser.parse_args()
    binary = pathlib.Path(args.binary).resolve()
    with tempfile.TemporaryDirectory(prefix="akula-multi-loopback-") as tmp:
        root = pathlib.Path(tmp)
        server_key, good_key = root / "server.key", root / "good.key"
        identity = lambda key: call(binary, "identity", "--key", key).stdout.strip().split()[-1]
        server_id, good_id = identity(server_key), identity(good_key)
        book = root / "test.bin"
        book.write_bytes(bytes(range(256)) * 256)
        server = subprocess.Popen(
            [str(binary), "serve", "--key", str(server_key), "--book", str(book),
             "--allow", good_id, "--bind", "127.0.0.1:0",
             "--quota-state", str(root / "quota"), "--global-quota", "100000",
             "--peer-quota", "100000"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        try:
            ready, _, _ = select.select([server.stdout], [], [], 30)
            assert ready, "server startup timed out"
            line = server.stdout.readline().strip()
            match = re.fullmatch(r"READY id=([a-z0-9]+) addr=([^ ]+) hash=([a-f0-9]{64})", line)
            assert match and match.group(1) == server_id, line
            addr = match.group(2)
            def stranger(i):
                key = root / f"stranger-{i}.key"
                identity(key)
                return call(binary, "fetch", "--key", key, "--bind",
                            f"127.0.0.{i+2}:0", "--remote-id", server_id,
                            "--remote-addr", addr, "--stage", root / f"part-{i}",
                            "--output", root / f"out-{i}", timeout=22)
            with concurrent.futures.ThreadPoolExecutor(max_workers=12) as executor:
                futures = [executor.submit(stranger, i) for i in range(12)]
                time.sleep(0.25)
                good = call(binary, "fetch", "--key", good_key, "--remote-id",
                            server_id, "--remote-addr", addr, "--stage", root / "goodparts",
                            "--output", root / "goodout", timeout=25)
                unknown = [future.result(timeout=28) for future in futures]
            assert good.returncode == 0, good.stderr
            assert (root / "goodout").read_bytes() == book.read_bytes()
            assert all(p.returncode != 0 for p in unknown), "untrusted peer obtained file"
            assert all(not (root / f"out-{i}").exists() for i in range(12))
            print("MULTI_SOURCE_12 denied=12 known_download=ok "
                  "sources=127.0.0.2..127.0.0.13 scope=linux_loopback")
        finally:
            server.terminate()
            try:
                server.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.communicate(timeout=5)

if __name__ == "__main__":
    main()
