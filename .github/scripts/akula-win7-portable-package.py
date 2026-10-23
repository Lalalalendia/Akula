#!/usr/bin/env python3
"""Build a disposable Win7 portable trial from the actual Win7-target EXE.

No compilation or dependency downgrade. Fails closed on unexpected PE imports.
This is a LAB build, not a compatibility statement or production release.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import struct
import zipfile


def fail(reason: str) -> None:
    raise SystemExit("WIN7_LAB_FAIL_CLOSED: " + reason)


def u16(data: bytes | bytearray, offset: int) -> int:
    return struct.unpack_from("<H", data, offset)[0]


def u32(data: bytes | bytearray, offset: int) -> int:
    return struct.unpack_from("<I", data, offset)[0]


def find_import_dll_offsets(data: bytearray) -> list[tuple[int, bytes]]:
    if len(data) < 512 or data[:2] != b"MZ":
        fail("not a PE file")
    pe = u32(data, 0x3C)
    if data[pe:pe + 4] != b"PE\x00\x00":
        fail("missing PE signature")
    if u16(data, pe + 4) != 0x8664:
        fail("expected x86-64 PE")
    optional = pe + 24
    if u16(data, optional) != 0x20B:
        fail("expected PE32+ optional header")
    optional_size = u16(data, pe + 20)
    sections = u16(data, pe + 6)
    header_size = u32(data, optional + 60)
    table = optional + optional_size
    import_rva = u32(data, optional + 112 + 8)
    if not import_rva or not sections:
        fail("missing import directory")

    def rva_offset(rva: int) -> int:
        if rva < header_size and rva < len(data):
            return rva
        for i in range(sections):
            base = table + 40 * i
            virt_size = u32(data, base + 8)
            virt_addr = u32(data, base + 12)
            disk_size = u32(data, base + 16)
            disk_addr = u32(data, base + 20)
            delta = rva - virt_addr
            if 0 <= delta < max(virt_size, disk_size) and delta < disk_size:
                offset = disk_addr + delta
                if offset < len(data):
                    return offset
        fail("import RVA is not backed by file bytes")

    result = []
    for i in range(512):
        offset = rva_offset(import_rva + 20 * i)
        descriptor = struct.unpack_from("<IIIII", data, offset)
        if descriptor == (0, 0, 0, 0, 0):
            return result
        dll_offset = rva_offset(descriptor[3])
        end = data.find(b"\x00", dll_offset, min(dll_offset + 256, len(data)))
        if end == -1:
            fail("unterminated import DLL name")
        result.append((dll_offset, bytes(data[dll_offset:end]).lower()))
    fail("import descriptor table not terminated")


README = """AKULA WIN7 LAB - PORTABLE TEST, NOT A RELEASE

For Windows 7 SP1 64-bit only. EXE is unsigned and untested on real Win7.
This is the experimental Akula PR #667 binary, not the public Akula release.

1. Extract all files to a NEW folder on a test PC/VM.
2. Double-click START-WIN7-LAB.cmd (not the public installer).
3. Create, edit and save a disposable test note.
4. Close the app, reopen it, check the note and send any error screenshot.

The launcher uses a test vault and separate data directory beside this file.
Do not use private files, network sync, or normal automatic updating.
Do not overwrite your production Akula installation.

The experimental EXE was compiled with the Win7 target, and the import
descriptor for 2 COM functions was remapped from combase.dll to ole32.dll.
The resulting import table is audited after patching, not just before.

System may need Windows 7 SP1, its supported system updates, UCRT KB2999226,
and a suitable Microsoft Visual C++ 64-bit runtime. Avoid third-party DLL sites.
Report exact missing-DLL/function names on any startup error.
"""


LAUNCHER = """@echo off
setlocal
set "ROOT=%~dp0"
if not exist "%ROOT%vault" mkdir "%ROOT%vault"
if not exist "%ROOT%state" mkdir "%ROOT%state"
"%ROOT%akula-app.exe" --vault "%ROOT%vault" --data-dir "%ROOT%state"
"""


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--exe", type=Path, required=True)
    p.add_argument("--audit", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--revision", required=True)
    args = p.parse_args()
    report = json.loads(args.audit.read_text(encoding="utf-8"))
    expected = {"CoCreateFreeThreadedMarshaler", "CoTaskMemFree"}
    if set(report.get("imports", {}).get("combase.dll", [])) != expected:
        fail("unexpected combase.dll exports: cannot safely remap")
    if report.get("known_post_win7_imports"):
        fail("known Windows 8+ static imports remain")

    data = bytearray(args.exe.read_bytes())
    descriptors = find_import_dll_offsets(data)
    combase = [offset for offset, dll in descriptors if dll == b"combase.dll"]
    if len(combase) != 1:
        fail("expected exactly one combase.dll import descriptor")
    offset = combase[0]
    before = b"combase.dll\x00"
    after = b"ole32.dll\x00\x00\x00"
    if len(before) != len(after) or data[offset:offset + len(before)] != before:
        fail("unexpected name bytes at import descriptor")
    data[offset:offset + len(before)] = after

    updated_dlls = [name for _, name in find_import_dll_offsets(data)]
    if b"combase.dll" in updated_dlls or b"ole32.dll" not in updated_dlls:
        fail("COMBASE import descriptor rewrite not verified")
    args.out.mkdir(parents=True, exist_ok=True)
    exe = args.out / "akula-app.exe"
    exe.write_bytes(data)
    digest = hashlib.sha256(data).hexdigest()
    (args.out / "README-WIN7-LAB.txt").write_text(README, encoding="utf-8")
    (args.out / "START-WIN7-LAB.cmd").write_text(LAUNCHER, encoding="ascii")
    provenance = (
        "source_sha=" + args.revision + "\n"
        "patched_exe_sha256=" + digest + "\n"
        "patched_exe_bytes=" + str(len(data)) + "\n"
        "pe_rewrite=combase.dll to ole32.dll for 2 verified Co* imports\n"
        "runtime_win7_verified=false\n"
        "production_release=false\n"
    )
    (args.out / "PROVENANCE.txt").write_text(provenance, encoding="ascii")
    archive = args.out / "Akula-Win7-LAB-portable.zip"
    with zipfile.ZipFile(archive, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for name in ("akula-app.exe", "START-WIN7-LAB.cmd", "README-WIN7-LAB.txt", "PROVENANCE.txt"):
            zf.write(args.out / name, arcname=name)
    with zipfile.ZipFile(archive) as zf:
        if zf.testzip() is not None or len(zf.namelist()) != 4:
            fail("portable archive integrity check failed")
    print("WIN7_LAB_PORTABLE_READY", archive, digest, archive.stat().st_size)


if __name__ == "__main__":
    main()
