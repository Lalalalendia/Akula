#!/usr/bin/env python3
"""End-to-end multicast mDNS discovery plus authenticated QUIC over an offline veth.

Two independent Linux network namespaces have no default route. This exercises
serve-mdns and ping-discovered; plain direct LAN tests live in a separate file.
It is a virtual network test, not a physical two-computer acceptance test.
"""
import os
import pathlib
import re
import subprocess
import sys
import time
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "staya-lan"

# Unique on an isolated GitHub Actions runner, within Linux IFNAMSIZ=16.
TAG = f"{os.getpid() % 100000:05d}"
NS_A = f"akula-mdns-a-{TAG}"
NS_B = f"akula-mdns-b-{TAG}"
LINK_A = f"ama{TAG}"
LINK_B = f"amb{TAG}"
IP_A = "10.231.43.1"
IP_B = "10.231.43.2"
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


def identity(folder, label):
    key_path = pathlib.Path(folder) / (label + ".key")
    result = subprocess.run([str(BIN), "gen-key-file", str(key_path)], capture_output=True, text=True, timeout=15, check=True)
    match = re.search(r"^endpoint_id=(\S+)$", result.stdout, re.M)
    if not match:
        raise AssertionError("identity generation failed")
    return match.group(1), "@" + str(key_path)


def sanitized(output, secrets):
    for secret in secrets:
        output = output.replace(secret, "[secret-redacted]")
    return output[-1200:]


def verified_pin(folder, identity_key, identity_id, label):
    """Create a local trust record only after exact identity confirmation."""
    contact = pathlib.Path(folder) / (label + ".contact")
    trusted = pathlib.Path(folder) / (label + ".trusted")
    exported = subprocess.run(
        [str(BIN), "show-contact", identity_key],
        capture_output=True, text=True, check=True, timeout=15
    )
    contact.write_bytes(exported.stdout.encode("utf-8"))
    subprocess.run(
        [str(BIN), "confirm-contact", str(contact), identity_id, str(trusted)],
        capture_output=True, text=True, check=True, timeout=15
    )
    return str(trusted)


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
        # An offline network namespace has no default route. IPv4 multicast
        # (224.0.0.251:5353) requires an explicit link-local route.
        ip("-n", ns, "route", "add", "224.0.0.0/4", "dev", link, "src", address)
        multicast_route = inside(ns, "ip", "route", "get", "224.0.0.251")
        if multicast_route.returncode != 0 or link not in multicast_route.stdout:
            raise AssertionError("mDNS multicast route not reachable on veth")
        if ip("-n", ns, "route", "show", "default").stdout.strip():
            raise AssertionError(f"unexpected default route in {ns}")
        if inside(ns, "ip", "route", "get", "1.1.1.1").returncode == 0:
            raise AssertionError(f"unexpected external route in {ns}")


def probe_multicast_link():
    """Prove multicast delivery without involving the mDNS library."""
    receiver = r"""
import socket, sys
group, local = "224.0.0.251", sys.argv[1]
port = int(sys.argv[2])
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
sock.bind(("", port))
sock.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                socket.inet_aton(group) + socket.inet_aton(local))
sock.settimeout(6)
data, _ = sock.recvfrom(256)
if data != b"staya-multicast-probe-v0":
    sys.exit(2)
"""
    sender = r"""
import socket, sys
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF,
                socket.inet_aton(sys.argv[1]))
sock.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 1)
sock.sendto(b"staya-multicast-probe-v0", ("224.0.0.251", int(sys.argv[2])))
"""
    for port in (53921, 5353):
        waiting = subprocess.Popen(
            ["sudo", "-n", "ip", "netns", "exec", NS_A, sys.executable, "-c",
             receiver, IP_A, str(port)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            time.sleep(0.6)
            sent = inside(NS_B, sys.executable, "-c", sender, IP_B, str(port), timeout=8)
            if sent.returncode != 0:
                raise AssertionError("test multicast sender failed: " + sent.stderr[-500:])
            _, err = waiting.communicate(timeout=9)
            if waiting.returncode != 0:
                raise AssertionError(
                    f"no IPv4 multicast UDP on port {port} over veth: " + err[-500:])
            print(f"STAYA_MULTICAST_LINK_PASS port={port}", flush=True)
        finally:
            if waiting.poll() is None:
                waiting.kill()
                waiting.communicate(timeout=5)


def start_mdns_captures(folder):
    """Capture only UDP/5353 packet headers, never application/key payload."""
    result = []
    for ns, link, label in [(NS_A, LINK_A, "a"), (NS_B, LINK_B, "b")]:
        path = pathlib.Path(folder) / ("mdns-" + label + ".txt")
        with path.open("w", encoding="utf-8") as output:
            proc = subprocess.Popen(
                ["sudo", "-n", "ip", "netns", "exec", ns,
                 "tcpdump", "-n", "-l", "-v", "-i", link, "udp", "port", "5353"],
                stdout=output, stderr=subprocess.PIPE, text=True)
        result.append((label, path, proc))
    time.sleep(0.7)
    for label, path, proc in result:
        if proc.poll() is not None:
            _, error = proc.communicate(timeout=5)
            raise AssertionError("mDNS header capture failed on " + label + ": " + error[-500:])
    return result


def report_mdns_captures(captures):
    """Summarize traffic direction without exposing host names or endpoint IDs."""
    for label, path, proc in captures:
        if proc.poll() is None:
            proc.terminate()
        try:
            proc.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.communicate(timeout=5)
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        # With -v tcpdump includes IP TTL and DNS record types. Parse locally
        # but publish *only aggregate counts* to avoid exposing endpoint IDs.
        blocks = []
        current = []
        for line in lines:
            if re.match(r"^\d\d:\d\d:\d\d\.\d+ IP(?:\s|\()", line):
                if current:
                    blocks.append(" ".join(current))
                current = [line]
            elif current:
                current.append(line)
        if current:
            blocks.append(" ".join(current))
        from_a = sum(IP_A + ".5353 >" in block for block in blocks)
        from_b = sum(IP_B + ".5353 >" in block for block in blocks)
        ttl255 = sum("ttl 255" in block for block in blocks)
        service = sum("_akula-staya._udp.local." in block for block in blocks)
        ptr_question = sum("PTR?" in block for block in blocks)
        dns_response = sum("PTR " in block or "TXT " in block or "SRV " in block for block in blocks)
        print(
            f"STAYA_MDNS_WIRE ns={label} packets={len(blocks)} from_a={from_a} from_b={from_b} "
            f"ttl255={ttl255} service_mentions={service} ptr_question={ptr_question} dns_response={dns_response}",
            flush=True)


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
    with tempfile.TemporaryDirectory(prefix='staya-private-') as folder:
        return run_with_keys(folder)


def run_with_keys(folder):
    a_id, a_key = identity(folder, 'a')
    b_id, b_key = identity(folder, 'b')
    unknown_id, unknown_key = identity(folder, 'unknown')
    a_pin = verified_pin(folder, a_key, a_id, "a-verified")
    b_pin = verified_pin(folder, b_key, b_id, "b-verified")
    unknown_pin = verified_pin(folder, unknown_key, unknown_id, "unknown-verified")
    peer_address = f"{IP_B}:{PORT}"
    server = None
    captures = []
    try:
        setup_network()
        probe_multicast_link()
        captures = start_mdns_captures(folder)
        server = subprocess.Popen(
            ["sudo", "-n", "ip", "netns", "exec", NS_B, str(BIN),
             "serve-mdns-trusted", b_key, a_pin, peer_address, IP_B],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True
        )
        time.sleep(2)
        if server.poll() is not None:
            _, err = server.communicate(timeout=5)
            raise AssertionError("server exited: " +
                                 sanitized(err, [a_key, b_key, unknown_key]))

        # The client knows only B's enrolled public key, not B's IP.
        good = inside(NS_A, BIN, "ping-discovered-trusted", a_key, b_pin, "12", timeout=50)
        if good.returncode != 0 or "via=mdns relay=disabled" not in good.stdout:
            # Publisher-side monitor reports whether DNS-SD was actually
            # announced and whether its daemon encountered any errors.
            # It normally stays silent in GitHub Actions because stderr is
            # redirected to a pipe. Collect that pipe *only on failure*.
            if server.poll() is None:
                server.terminate()
            try:
                _, publisher_error = server.communicate(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                _, publisher_error = server.communicate(timeout=5)
            raise AssertionError(
                "mDNS-resolved QUIC ping failed: " +
                sanitized(good.stderr, [a_key, b_key, unknown_key]) +
                " publisher: " +
                sanitized(publisher_error, [a_key, b_key, unknown_key])
            )

        # A foreign secret key can discover B's public record but must never
        # complete the authenticated QUIC exchange.
        denied = inside(NS_A, BIN, "ping-discovered-trusted", unknown_key, b_pin, "8",
                        timeout=55)
        if denied.returncode == 0:
            raise AssertionError("unknown peer was accepted after mDNS")
        if "all discovered addresses failed QUIC verification" not in denied.stderr:
            raise AssertionError(
                "unknown key did not reach QUIC authentication: " +
                sanitized(denied.stderr, [a_key, b_key, unknown_key])
            )

        # An unapproved/other EndpointId must not resolve to B at all.
        other = inside(NS_A, BIN, "ping-discovered-trusted", a_key, unknown_pin, "3",
                       timeout=15)
        if other.returncode == 0 or "no permitted peers were discovered" not in other.stderr:
            raise AssertionError("mDNS allowed an unrelated endpoint identity")
        # A missing trusted record must not fall back to unconfirmed mDNS.
        missing = inside(
            NS_A, BIN, "ping-discovered-trusted", a_key,
            str(pathlib.Path(folder) / "missing.trusted"), "3", timeout=12
        )
        if missing.returncode == 0:
            raise AssertionError("missing trusted record bypassed enrollment")
        print("STAYA_MDNS_TRUSTED_PASS: offline trusted contact mDNS and authenticated QUIC; foreign keys rejected")
    finally:
        report_mdns_captures(captures)
        kill_server(server)
        cleanup()


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, OSError, subprocess.CalledProcessError,
            subprocess.TimeoutExpired) as exc:
        print(f"STAYA_NETNS_FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
