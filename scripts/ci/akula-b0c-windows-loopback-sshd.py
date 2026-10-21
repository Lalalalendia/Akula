#!/usr/bin/env python3
"""Disposable Windows-only loopback SSH server used to falsify real signed KEX.

NEVER run on the network: binds 127.0.0.1 with an ephemeral CI-only Ed25519
host key. No shells, forwarding, files, sudo, user credentials or installer.
Paramiko is a TEST dependency only; Akula Rust binaries do not depend on it.
"""
import json
import os
import pathlib
import socket
import subprocess
import sys
import threading
import time

import paramiko


class ReadOnlyServer(paramiko.ServerInterface):
    def __init__(self, expected_client_key):
        self.command = None
        self.exec_requested = threading.Event()
        self.expected_client_key = expected_client_key

    def check_auth_publickey(self, username, key):
        if (
            username == "ubuntu"
            and key.get_name() == self.expected_client_key.get_name()
            and key.asbytes() == self.expected_client_key.asbytes()
        ):
            return paramiko.AUTH_SUCCESSFUL
        return paramiko.AUTH_FAILED

    def get_allowed_auths(self, username):
        return "publickey" if username == "ubuntu" else ""

    def check_channel_request(self, kind, chanid):
        return (
            paramiko.OPEN_SUCCEEDED
            if kind == "session"
            else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED
        )

    def check_channel_exec_request(self, channel, command):
        if command == b"true" or (
            command.startswith(b"LC_ALL=C; export LC_ALL; ")
            and b"AKULA_B0_OS=" in command
            and b"AKULA_B0_DISK=" in command
            and len(command) < 2048
        ):
            self.command = command
            self.exec_requested.set()
            return True
        return False


FACTS = (
    b"AKULA_B0_OS=ID=ubuntu\n"
    b"AKULA_B0_ARCH=x86_64\n"
    b"AKULA_B0_SYSTEMD=yes\n"
    b"AKULA_B0_SUDO=yes\n"
    b"AKULA_B0_CPU=4\n"
    b"AKULA_B0_RAM=8192\n"
    b"AKULA_B0_DISK=30720\n"
)


def handle(client, key, expected_client_key):
    transport = None
    try:
        transport = paramiko.Transport(client)
        transport.add_server_key(key)
        server = ReadOnlyServer(expected_client_key)
        transport.start_server(server=server)
        chan = transport.accept(10)
        if chan is None or not server.exec_requested.wait(10):
            return
        result = b"" if server.command == b"true" else FACTS
        if result:
            chan.sendall(result)
        chan.send_exit_status(0)
        chan.shutdown_write()
        time.sleep(0.1)
    except (OSError, EOFError, paramiko.SSHException) as exc:
        print(f"LOCAL_SSH_TEST_CONNECTION_REFUSED: {type(exc).__name__}", file=sys.stderr)
    finally:
        if transport is not None:
            transport.close()
        client.close()


def make_key(path):
    subprocess.run(
        ["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(path)],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
        timeout=30,
    )
    return paramiko.Ed25519Key.from_private_key_file(str(path))


def main():
    if len(sys.argv) != 2 or os.environ.get("GITHUB_ACTIONS") != "true":
        raise SystemExit("CI-only fixture expects one isolated output directory")
    out = pathlib.Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=False)
    key_a = make_key(out / "a")
    key_b = make_key(out / "b")
    client_key = make_key(out / "client")
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
    server.bind(("127.0.0.1", 0))
    server.listen(8)
    server.settimeout(2)
    port = server.getsockname()[1]
    data = {
        "port": port,
        "public_a": f"{key_a.get_name()} {key_a.get_base64()}",
        "public_b": f"{key_b.get_name()} {key_b.get_base64()}",
    }
    tmp = out / "fixture.tmp"
    tmp.write_text(json.dumps(data), encoding="utf-8")
    os.replace(tmp, out / "fixture.json")
    print("B0C_LOCALHOST_SSH_SERVER_READY", flush=True)
    while True:
        try:
            client, address = server.accept()
        except socket.timeout:
            continue
        if address[0] != "127.0.0.1":
            client.close()
            continue
        threading.Thread(
            target=handle, args=(client, key_a, client_key), daemon=True
        ).start()


if __name__ == "__main__":
    main()
