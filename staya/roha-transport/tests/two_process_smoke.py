#!/usr/bin/env python3
"""Independent OS-process smoke test for staya-lan, loopback only."""
import os
import pathlib
import re
import socket
import subprocess
import sys
import time

BIN = pathlib.Path(__file__).resolve().parents[1] / "target" / "debug" / ("staya-lan.exe" if os.name == "nt" else "staya-lan")

def make_identity():
    result = subprocess.run([str(BIN), "gen-key"], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15, check=True)
    id_match = re.search(r"^endpoint_id=(\S+)$", result.stdout, re.M)
    key_match = re.search(r"^secret_hex=([0-9a-f]{64})$", result.stdout, re.M)
    if not id_match or not key_match:
        raise AssertionError("identity generation failed")
    return id_match.group(1), key_match.group(1)

def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]

def main():
    a_id, a_key = make_identity()
    b_id, b_key = make_identity()
    wrong_id, _ = make_identity()
    port = free_port()
    peer = f"127.0.0.1:{port}"
    server = subprocess.Popen([str(BIN), "serve", b_key, a_id, peer],
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True)
    try:
        time.sleep(1)
        if server.poll() is not None:
            raise AssertionError("server exited before client connection")
        def ping(target, expect_ok):
            result = subprocess.run([str(BIN), "ping", a_key, target, peer],
                                    text=True, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=22)
            if (result.returncode == 0) != expect_ok:
                raise AssertionError(f"unexpected ping result: exit={result.returncode}")
            if expect_ok and "direct_lan_ping_ok" not in result.stdout:
                raise AssertionError("missing direct LAN success marker")
        ping(b_id, True)
        ping(wrong_id, False)
        print("staya two-process loopback: PASS; valid endpoint accepted, wrong endpoint denied")
    finally:
        server.terminate()
        try:
            server.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.communicate(timeout=5)

if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"staya process smoke: FAIL: {error}", file=sys.stderr)
        sys.exit(1)
