"""Offline B1-E2E-07 helper manifest and externally signed frame preparation.

This tool NEVER generates or loads private signing keys. The Ed25519 signature
must be produced separately, offline, under an independently managed key.
A valid frame is only DATA for the existing root_stage_v1.py HOLD verifier;
neither this CLI nor the resulting frame installs or executes a root helper.
"""
import argparse
import hashlib
import io
import os
import pathlib
import re
import stat
import struct
import sys
import tempfile
import time

import root_stage_v1 as stage

MAX_U32 = (1 << 32) - 1
MAX_U64 = (1 << 64) - 1
SIGNER = stage.EXPECTED_SIGNER.encode("ascii")


class ReleaseHold(ValueError):
    """Fail closed without creating an authorized installer."""


def _uint(value, maximum):
    if isinstance(value, bool) or not isinstance(value, int) or not 0 <= value <= maximum:
        raise ReleaseHold("invalid unsigned integer")
    return value


def parse_version(raw):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", raw, re.ASCII):
        raise ReleaseHold("version must be major.minor.patch")
    fields = tuple(_uint(int(part), MAX_U32) for part in raw.split("."))
    if str(fields[0]) + "." + str(fields[1]) + "." + str(fields[2]) != raw:
        raise ReleaseHold("noncanonical version")
    return fields


def read_elf(path):
    """Bounded, stable regular local ELF; NOT proof of installed root custody."""
    try:
        descriptor = os.open(
            path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError as exc:
        raise ReleaseHold("unsafe or unavailable local ELF input") from exc
    with os.fdopen(descriptor, "rb") as stream:
        before = os.fstat(stream.fileno())
        if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                or not 64 <= before.st_size <= stage.MAX_HELPER_BYTES):
            raise ReleaseHold("invalid local helper inode or size")
        data = stream.read(stage.MAX_HELPER_BYTES + 1)
        after = os.fstat(stream.fileno())
    if (len(data) != before.st_size or before.st_dev != after.st_dev
            or before.st_ino != after.st_ino or before.st_size != after.st_size
            or before.st_mtime_ns != after.st_mtime_ns
            or before.st_ctime_ns != after.st_ctime_ns):
        raise ReleaseHold("local helper changed during read")
    if data[:7] != b"\x7fELF\x02\x01\x01" or data[18:20] != b"\x3e\x00":
        raise ReleaseHold("not a Linux x86-64 ELF")
    return data



def read_bounded_metadata(path, maximum):
    """No symlink, hardlink, unbounded read or changed local input."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    except OSError as exc:
        raise ReleaseHold("unsafe or missing metadata input") from exc
    with os.fdopen(fd, "rb") as source:
        before = os.fstat(source.fileno())
        if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1
                or before.st_size <= 0 or before.st_size > maximum):
            raise ReleaseHold("wrong metadata inode or size")
        body = source.read(maximum + 1)
        after = os.fstat(source.fileno())
    if (len(body) != before.st_size or before.st_dev != after.st_dev
            or before.st_ino != after.st_ino or before.st_size != after.st_size
            or before.st_mtime_ns != after.st_mtime_ns
            or before.st_ctime_ns != after.st_ctime_ns):
        raise ReleaseHold("metadata input changed while reading")
    return body


def prepare_manifest(elf, version, sequence, not_before, expires):
    """Create exactly the canonical B1a bytes, without any signature."""
    major, minor, patch = parse_version(version)
    _uint(sequence, MAX_U64)
    _uint(not_before, MAX_U64)
    _uint(expires, MAX_U64)
    if sequence == 0 or expires <= not_before:
        raise ReleaseHold("release sequence or validity window invalid")
    if not 64 <= len(elf) <= stage.MAX_HELPER_BYTES:
        raise ReleaseHold("helper size invalid")
    if elf[:7] != b"\x7fELF\x02\x01\x01" or elf[18:20] != b"\x3e\x00":
        raise ReleaseHold("not a Linux x86-64 ELF")
    manifest = (stage.B1A + b"\x01\x01"
                + struct.pack(">IIIQQQQ", major, minor, patch,
                              sequence, len(elf), not_before, expires)
                + hashlib.sha256(elf).digest() + bytes([len(SIGNER)]) + SIGNER)
    try:
        size, digest = stage.decode_manifest(manifest, not_before)
    except stage.Hold as exc:
        raise ReleaseHold("root-stage manifest contract refused") from exc
    if size != len(elf) or digest != hashlib.sha256(elf).digest():
        raise ReleaseHold("manifest/ELF mismatch")
    return manifest


def independent_public_hex(raw):
    if not re.fullmatch(r"[0-9a-f]{64}", raw, re.ASCII):
        raise ReleaseHold("expected independently reviewed 64-char lowercase Ed25519 key")
    key = bytes.fromhex(raw)
    if key == bytes(32):
        raise ReleaseHold("zero public signing key")
    return key


def assemble_verified_frame(elf, manifest, signature, trusted_public, now_unix):
    """Verify detached signature and exact ELF before emitting a B1a stage frame.

    The public key MUST be obtained outside the candidate bundle (e.g. an
    operator-approved release-key record). A key copied from an attacker
    supplied manifest or archive cannot establish independent trust.
    """
    _uint(now_unix, MAX_U64)
    if len(signature) != 64 or len(manifest) == 0 or len(manifest) > 256:
        raise ReleaseHold("wrong detached signature or manifest length")
    if len(trusted_public) != 32 or trusted_public == bytes(32):
        raise ReleaseHold("independent public key is required")
    try:
        size, digest = stage.decode_manifest(manifest, now_unix)
    except stage.Hold as exc:
        raise ReleaseHold("invalid canonical B1a manifest") from exc
    if (len(elf) != size or hashlib.sha256(elf).digest() != digest
            or elf[:7] != b"\x7fELF\x02\x01\x01"
            or elf[18:20] != b"\x3e\x00"):
        raise ReleaseHold("signed manifest differs from exact Linux ELF")
    frame = (stage.WIRE + struct.pack(">H", len(manifest))
             + manifest + signature + elf)
    # Reuse the existing exact first-root signature verifier. All scratch
    # files are temporary and never published as an authorized root stage.
    with tempfile.TemporaryDirectory(prefix="akula-b1d0-offline-review-") as temporary:
        try:
            _, _, verified_sha = stage.receive_verified(
                io.BytesIO(frame), trusted_public, now_unix,
                os.path.join(temporary, "scratch"))
        except (OSError, stage.Hold) as exc:
            raise ReleaseHold("detached Ed25519 signature or stage verification failed") from exc
    if verified_sha != digest:
        raise ReleaseHold("verified helper hash mismatch")
    return frame


def write_new(path, data):
    """One-use local output, no symlink or accidental overwrite."""
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC
    descriptor = os.open(path, flags, 0o600)
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as target:
            target.write(data)
            target.flush()
            os.fsync(descriptor)
    except BaseException:
        os.close(descriptor)
        pathlib.Path(path).unlink(missing_ok=True)
        raise
    else:
        os.close(descriptor)


def main(argv=None):
    parser = argparse.ArgumentParser(
        description="Prepare or verify B1a helper data OFFLINE; never signs or installs")
    command = parser.add_subparsers(dest="operation", required=True)
    prepare = command.add_parser("prepare", help="prepare unsigned signable manifest")
    prepare.add_argument("--elf", required=True)
    prepare.add_argument("--version", required=True)
    prepare.add_argument("--sequence", type=int, required=True)
    prepare.add_argument("--not-before", type=int, required=True)
    prepare.add_argument("--expires", type=int, required=True)
    prepare.add_argument("--out-manifest", required=True)
    frame = command.add_parser("frame", help="verify external signature, create inert stage frame")
    frame.add_argument("--elf", required=True)
    frame.add_argument("--manifest", required=True)
    frame.add_argument("--signature", required=True)
    frame.add_argument("--trusted-pubkey-hex", required=True)
    frame.add_argument("--out-frame", required=True)
    frame.add_argument("--now-unix", type=int, default=None)
    args = parser.parse_args(argv)
    try:
        elf = read_elf(args.elf)
        if args.operation == "prepare":
            manifest = prepare_manifest(
                elf, args.version, args.sequence, args.not_before, args.expires)
            write_new(args.out_manifest, manifest)
            print("AKULA_B1D0_SIGNABLE_MANIFEST_ONLY_NOT_SIGNED")
        else:
            manifest = read_bounded_metadata(args.manifest, 256)
            signature = read_bounded_metadata(args.signature, 64)
            key = independent_public_hex(args.trusted_pubkey_hex)
            current = int(time.time()) if args.now_unix is None else args.now_unix
            result = assemble_verified_frame(elf, manifest, signature, key, current)
            write_new(args.out_frame, result)
            print("AKULA_B1D0_SIGNATURE_VERIFIED_FRAME_ONLY_INSTALL_HOLD")
        return 0
    except (OSError, ReleaseHold, ValueError) as exc:
        print("AKULA_B1D0_OFFLINE_RELEASE_HOLD: " + str(exc), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
