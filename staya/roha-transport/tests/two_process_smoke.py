#!/usr/bin/env python3
"""Independent OS-process smoke test for staya-lan, loopback only."""
import os
import pathlib
import re
import socket
import subprocess
import sys
import time
import tempfile

BIN = pathlib.Path(__file__).resolve().parents[1] / "target" / "debug" / ("staya-lan.exe" if os.name == "nt" else "staya-lan")

def make_identity(folder, label):
    key_path = pathlib.Path(folder) / (label + ".key")
    result = subprocess.run([str(BIN), "gen-key-file", str(key_path)], text=True, capture_output=True, timeout=15, check=True)
    match = re.search(r"^endpoint_id=(\S+)$", result.stdout, re.M)
    if not match:
        raise AssertionError("identity generation failed")
    return match.group(1), "@" + str(key_path)


def free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]

def main():
    with tempfile.TemporaryDirectory(prefix='staya-private-') as folder:
        return run_with_keys(folder)


def run_with_keys(folder):
    a_id, a_key = make_identity(folder, 'a')
    b_id, b_key = make_identity(folder, 'b')
    wrong_id, _ = make_identity(folder, 'wrong')
    # Public exchange does not grant trust; full identity must match.
    def pin(peer_id, peer_key, name):
        contact = pathlib.Path(folder) / (name + ".contact")
        trusted = pathlib.Path(folder) / (name + ".trusted")
        exported = subprocess.run([str(BIN), "show-contact", peer_key],
                                  capture_output=True, text=True, check=True, timeout=15)
        contact.write_text(exported.stdout, encoding="utf-8")
        examined = subprocess.run([str(BIN), "inspect-contact", str(contact)],
                                  capture_output=True, text=True, check=True, timeout=15)
        if "not_enrolled=true" not in examined.stdout:
            raise AssertionError("contact unexpectedly became trusted")
        rejected = subprocess.run([str(BIN), "confirm-contact", str(contact), wrong_id, str(trusted)],
                                  capture_output=True, text=True, timeout=15)
        if rejected.returncode == 0 or trusted.exists():
            raise AssertionError("incorrect identity was trusted")
        subprocess.run([str(BIN), "confirm-contact", str(contact), peer_id, str(trusted)],
                       capture_output=True, text=True, check=True, timeout=15)
        overwritten = subprocess.run([str(BIN), "confirm-contact", str(contact), peer_id, str(trusted)],
                                     capture_output=True, text=True, timeout=15)
        if overwritten.returncode == 0:
            raise AssertionError("trusted identity was overwritten")
        return str(trusted)

    b_pin = pin(b_id, b_key, "b")
    a_pin = pin(a_id, a_key, "a")
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
                diagnostic = result.stderr.replace(a_key, "[secret-redacted]").replace(b_key, "[secret-redacted]")
                raise AssertionError(f"unexpected ping result: exit={result.returncode}; client={diagnostic[-1000:]!r}; server_running={server.poll() is None}")
            if expect_ok and "direct_lan_ping_ok" not in result.stdout:
                raise AssertionError("missing direct LAN success marker")
        ping(b_id, True)
        trusted = subprocess.run([str(BIN), "ping-trusted", a_key, b_pin, peer], capture_output=True, text=True, timeout=22)
        if trusted.returncode != 0 or "direct_lan_ping_ok" not in trusted.stdout:
            raise AssertionError("confirmed trusted connection did not succeed")
        missing = subprocess.run([str(BIN), "ping-trusted", a_key, str(pathlib.Path(folder) / "missing.trusted"), peer], capture_output=True, text=True, timeout=15)
        if missing.returncode == 0:
            raise AssertionError("missing pin was accepted")
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
