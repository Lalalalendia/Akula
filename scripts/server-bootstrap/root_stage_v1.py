"""Akula B1-E2E-07 fixed first-root staging, no package execution.

Future Windows sender must transmit this exact bundled source using a pinned SSH
session to /usr/bin/python3 -I -c, *after* explicit owner authorisation.
This source is never loaded from a user-writable remote file. The production
trust anchor below is intentionally absent until the actual release ceremony.
"""
import base64
import hashlib
import io
import os
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import time

# Must be shipped inside the signed Windows Akula, never read from the VPS,
# a remote release manifest, CLI arguments or environment variables.
PINNED_BOOTSTRAP_ED25519_HEX = ""
EXPECTED_SIGNER = "akula-root-bootstrap-v1"
B1A = b"Akula Node B1a signed release manifest v1\x00"
WIRE = b"AKULA_ROOT_STAGE_V1\x00"
MAX_HELPER_BYTES = 64 * 1024 * 1024
MAX_MANIFEST_BYTES = 256
MAX_VALIDITY_SECONDS = 90 * 24 * 60 * 60
PUB_SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
O_DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
O_NEW = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC


class Hold(Exception):
    """No privilege is granted and no executable may run."""


def read_exact(stream, n):
    out = bytearray()
    while len(out) < n:
        chunk = stream.read(min(65536, n - len(out)))
        if not chunk:
            raise Hold("truncated signed bootstrap frame")
        out.extend(chunk)
    return bytes(out)


def decode_manifest(body, now):
    if not (body.startswith(B1A) and len(body) <= MAX_MANIFEST_BYTES):
        raise Hold("noncanonical B1a manifest")
    rest = io.BytesIO(body[len(B1A):])
    if read_exact(rest, 2) != b"\x01\x01":
        raise Hold("unsupported B1a schema or target")
    major, minor, patch = struct.unpack(">III", read_exact(rest, 12))
    seq, size, start, expiry = struct.unpack(">QQQQ", read_exact(rest, 32))
    sha = read_exact(rest, 32)
    signer_len = read_exact(rest, 1)[0]
    signer = read_exact(rest, signer_len)
    if rest.read(1) or signer != EXPECTED_SIGNER.encode("ascii"):
        raise Hold("wrong signed helper identity")
    if (seq == 0 or size < 64 or size > MAX_HELPER_BYTES
            or sha == bytes(32) or not (start <= now < expiry)
            or expiry - start > MAX_VALIDITY_SECONDS):
        raise Hold("invalid version, bounds or time")
    return size, sha


def create_file(folder, leaf, body, mode=0o400):
    fd = os.open(leaf, O_NEW, 0o600, dir_fd=folder)
    try:
        with os.fdopen(fd, "wb", closefd=False) as stream:
            stream.write(body)
            stream.flush()
            os.fsync(fd)
        os.fchmod(fd, mode)
        os.fsync(fd)
    finally:
        os.close(fd)


def pinned_openssl_verify(folder, key):
    if len(key) != 32 or key == bytes(32):
        raise Hold("missing pinned release signing identity")
    tool = "/usr/bin/openssl"
    meta = os.lstat(tool)
    if not stat.S_ISREG(meta.st_mode) or meta.st_uid != 0 or meta.st_mode & 0o022:
        raise Hold("untrusted OS OpenSSL binary")
    spki = PUB_SPKI_PREFIX + key
    pem = (b"-----BEGIN PUBLIC KEY-----\n"
           + base64.encodebytes(spki).replace(b"\n", b"") + b"\n"
           + b"-----END PUBLIC KEY-----\n")
    create_file(folder, "release.pub", pem)
    args = [tool, "pkeyutl", "-verify", "-pubin", "-inkey", "release.pub",
            "-rawin", "-in", "manifest", "-sigfile", "signature"]
    try:
        completed = subprocess.run(args, cwd=f"/proc/self/fd/{folder}",
                                   stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, timeout=15, check=False,
                                   pass_fds=(folder,), env={"LANG": "C", "PATH": "/usr/bin:/bin"})
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise Hold("OS signature verifier unavailable") from exc
    if completed.returncode != 0:
        raise Hold("root-side Ed25519 release signature invalid")


def receive_verified(stream, trust_pubkey, now, scratch):
    if len(trust_pubkey) != 32 or trust_pubkey == bytes(32):
        raise Hold("production signing pin not provisioned")
    if read_exact(stream, len(WIRE)) != WIRE:
        raise Hold("wrong bootstrap frame domain")
    manifest_len = struct.unpack(">H", read_exact(stream, 2))[0]
    if manifest_len == 0 or manifest_len > MAX_MANIFEST_BYTES:
        raise Hold("manifest length exceeded")
    manifest = read_exact(stream, manifest_len)
    size, expected_sha = decode_manifest(manifest, now)
    signature = read_exact(stream, 64)
    os.mkdir(scratch, 0o700)
    fd = os.open(scratch, O_DIR)
    try:
        create_file(fd, "manifest", manifest)
        create_file(fd, "signature", signature)
        # Bounded input stream is data only. It is NEVER run as root here.
        helper = os.open("helper", O_NEW, 0o600, dir_fd=fd)
        try:
            hasher = hashlib.sha256()
            remaining = size
            while remaining:
                data = read_exact(stream, min(65536, remaining))
                hasher.update(data)
                written = 0
                while written < len(data):
                    count = os.write(helper, data[written:])
                    if count <= 0:
                        raise Hold("short write to protected staging")
                    written += count
                remaining -= len(data)
            if stream.read(1) != b"":
                raise Hold("unexpected trailing package bytes")
            os.fsync(helper)
            os.fchmod(helper, 0o400)
            os.fsync(helper)
        finally:
            os.close(helper)
        if hasher.digest() != expected_sha:
            raise Hold("signed package SHA-256 mismatch")
        with open(os.path.join(scratch, "helper"), "rb") as node:
            elf = node.read(20)
        if (len(elf) < 20 or elf[:4] != b"\x7fELF" or elf[4:7] != b"\x02\x01\x01"
                or elf[18:20] != b"\x3e\x00"):
            raise Hold("helper is not Linux x86-64 ELF")
        pinned_openssl_verify(fd, trust_pubkey)
        os.fsync(fd)
        return manifest, signature, expected_sha
    finally:
        os.close(fd)


def open_parent_root():
    # Traverse exact root-controlled ancestors; never follow symlinks.
    current = os.open("/", O_DIR)
    try:
        for name, mode in [("var", 0o755), ("lib", 0o755),
                           ("akula", 0o700), ("server-bootstrap", 0o700)]:
            try:
                next_fd = os.open(name, O_DIR, dir_fd=current)
            except FileNotFoundError:
                if name in ("var", "lib"):
                    raise Hold("unexpected missing system directory")
                os.mkdir(name, mode, dir_fd=current)
                next_fd = os.open(name, O_DIR, dir_fd=current)
            meta = os.fstat(next_fd)
            if meta.st_uid != 0 or stat.S_IMODE(meta.st_mode) != mode:
                os.close(next_fd)
                raise Hold("untrusted root directory ownership or mode")
            os.close(current)
            current = next_fd
        return current
    except Exception:
        os.close(current)
        raise


def copy_root_file(folder, leaf, source):
    fd = os.open(leaf, O_NEW, 0o600, dir_fd=folder)
    try:
        with open(source, "rb") as stream:
            while True:
                data = stream.read(65536)
                if not data:
                    break
                written = 0
                while written < len(data):
                    count = os.write(fd, data[written:])
                    if count <= 0:
                        raise Hold("short write to immutable destination")
                    written += count
        os.fsync(fd)
        os.fchmod(fd, 0o400)
        os.fsync(fd)
    finally:
        os.close(fd)


def publish_hold(parent_fd, scratch, manifest, signature, sha, trust_key):
    # O_EXCL directory creation: no overwrite, no automatic retries or repair.
    os.mkdir("bootstrap-v1", 0o700, dir_fd=parent_fd)
    dest = os.open("bootstrap-v1", O_DIR, dir_fd=parent_fd)
    try:
        for source, leaf in [("manifest", "manifest"), ("signature", "signature"),
                             ("helper", "helper.staged")]:
            copy_root_file(dest, leaf, os.path.join(scratch, source))
        create_file(dest, "release.ed25519", trust_key)
        create_file(dest, "HOLD", b"AKULA_ROOT_BOOTSTRAP_SIGNED_STAGE_ONLY_V1\n" + sha.hex().encode() + b"\n")
        os.fsync(dest)
        os.fsync(parent_fd)
    finally:
        os.close(dest)


def root_stage(stream):
    if os.geteuid() != 0:
        raise Hold("requires independently authorized root")
    if len(PINNED_BOOTSTRAP_ED25519_HEX) != 64:
        raise Hold("no production root bootstrap release key")
    try:
        pubkey = bytes.fromhex(PINNED_BOOTSTRAP_ED25519_HEX)
    except ValueError as exc:
        raise Hold("invalid pinned release key") from exc
    # Fixed transfer deadline prevents untrusted SSH stdin from holding a
    # privileged interpreter indefinitely. No secret prompts or retries.
    def expired(_signal, _frame):
        raise Hold("bootstrap receive deadline exceeded")
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(15 * 60)
    try:
        with tempfile.TemporaryDirectory(prefix="akula-root-stage-", dir="/run") as temp:
            scratch = os.path.join(temp, "candidate")
            manifest, signature, sha = receive_verified(stream, pubkey, int(time.time()), scratch)
            parent = open_parent_root()
            try:
                publish_hold(parent, scratch, manifest, signature, sha, pubkey)
            finally:
                os.close(parent)
    finally:
        signal.alarm(0)
    print("AKULA_ROOT_STAGE_VERIFIED_HOLD", flush=True)


if __name__ == "__main__":
    try:
        root_stage(sys.stdin.buffer)
    except (Hold, OSError, ValueError) as error:
        print("AKULA_ROOT_STAGE_HOLD: " + str(error), file=sys.stderr)
        sys.exit(2)
