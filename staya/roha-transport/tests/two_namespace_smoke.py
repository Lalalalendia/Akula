#!/usr/bin/env python3
"""Two Linux network namespaces joined only by a veth link.

The built staya-lan executable runs in different network namespaces without
a default route or internet. This tests an actual UDP exchange over a veth
link, not two peers bound to the same loopback network namespace.
"""
import os
import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "staya-lan"

# Unique on an isolated GitHub Actions runner, within Linux IFNAMSIZ=16.
TAG = f"{os.getpid() % 100000:05d}"
NS_A = f"akula-direct-a-{TAG}"
NS_B = f"akula-direct-b-{TAG}"
LINK_A = f"aka{TAG}"
LINK_B = f"akb{TAG}"
IP_A = "10.231.42.1"
IP_B = "10.231.42.2"
PORT = 42424


def ip(*args, check=True):
    return subprocess.run(
        ["sudo", "-n", "ip", *args],
        capture_output=True, text=True, timeout=15, check=check
    )


def inside(ns, *args, timeout=40):
    return subprocess.run(
        ["sudo", "-n", "ip", "netns", "exec", ns, *map(str, args)],
        capture_output=True, text=True, timeout=timeout
    )


def identity():
    result = subprocess.run(
        [str(BIN), "gen-key"], capture_output=True,
        text=True, timeout=15, check=True
    )
    peer = re.search(r"^endpoint_id=(\S+)$", result.stdout, re.M)
    secret = re.search(r"^secret_hex=([a-f0-9]{64})$", result.stdout, re.M)
    if not peer or not secret:
        raise AssertionError("unexpected identity response")
    return peer.group(1), secret.group(1)


def sanitized(output, secrets):
    for secret in secrets:
        output = output.replace(secret, "[secret-redacted]")
    return output[-1200:]


def setup_network():
    ip("netns", "add", NS_A)
    ip("netns", "add", NS_B)
    ip("link", "add", LINK_A, "type", "veth", "peer", "name", LINK_B)
    ip("link", "set", LINK_A, "netns", NS_A)
    ip("link", "set", LINK_B, "netns", NS_B)
    for ns, link, address in [(NS_A, LINK_A, IP_A), (NS_B, LINK_B, IP_B)]:
        ip("-n", ns, "addr", "add", f"{address}/30", "dev", link)
        ip("-n", ns, "link", "set", "lo", "up")
        ip("-n", ns, "link", "set", link, "up")
        if ip("-n", ns, "route", "show", "default").stdout.strip():
            raise AssertionError(f"unexpected default route in {ns}")
        if inside(ns, "ip", "route", "get", "1.1.1.1").returncode == 0:
            raise AssertionError(f"unexpected external route in {ns}")


def kill_server(server):
    if server is not None and server.poll() is None:
        server.terminate()
        try:
            server.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.communicate(timeout=5)
    # On runners where sudo exec forks a child, terminate any survivor before
    # deleting namespaces. Do not use non-namespaced process matching.
    for ns in [NS_A, NS_B]:
        pids = ip("netns", "pids", ns, check=False)
        for pid in pids.stdout.split():
            if pid.isdigit():
                subprocess.run(["sudo", "-n", "kill", "-TERM", pid],
                               capture_output=True, timeout=5)


def cleanup():
    for ns in [NS_A, NS_B]:
        ip("netns", "del", ns, check=False)


def main():
    a_id, a_key = identity()
    b_id, b_key = identity()
    _, unknown_key = identity()
    peer_address = f"{IP_B}:{PORT}"
    server = None
    try:
        setup_network()
        server = subprocess.Popen(
            ["sudo", "-n", "ip", "netns", "exec", NS_B, str(BIN),
             "serve", b_key, a_id, peer_address],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True
        )
        time.sleep(1)
        if server.poll() is not None:
            _, err = server.communicate(timeout=5)
            raise AssertionError("server exited: " +
                                 sanitized(err, [a_key, b_key, unknown_key]))

        def ping(source_key, should_work):
            result = inside(NS_A, BIN, "ping", source_key, b_id,
                            peer_address, timeout=40)
            if (result.returncode == 0) != should_work:
                raise AssertionError(
                    f"ping expected success={should_work} got={result.returncode}; " +
                    sanitized(result.stderr, [a_key, b_key, unknown_key]))
            if should_work and "direct_lan_ping_ok" not in result.stdout:
                raise AssertionError("successful exit without direct LAN marker")

        ping(a_key, True)
        ping(unknown_key, False)
        print("STAYA_NETNS_PASS: no internet routes; direct two-namespace QUIC; unknown peer denied")
    finally:
        kill_server(server)
        cleanup()


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, OSError, subprocess.CalledProcessError,
            subprocess.TimeoutExpired) as exc:
        print(f"STAYA_NETNS_FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
