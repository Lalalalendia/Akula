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

For Windows 7 SP1 64-bit only. Both EXEs are unsigned and untested on real Win7.
This is the experimental Akula PR #667 binary, not the public Akula release.

1. Extract all files to a NEW folder on a test PC/VM.
2. Double-click START-WIN7-LAB.cmd (not the public installer).
3. Create, edit and save a disposable test note.
4. Open Settings > Interface and check its text and controls.
5. Put Notepad over Akula, covering 25%, 50%, then 100% of the window.
   Move or close Notepad; all exposed pixels must be restored. Repeat
   the cover/uncover cycle at least 20 times.
6. Also test Alt+Tab, minimize/restore, window resize, Settings reopen.
   Compare basic Windows theme with Aero (if available).
7. Close Akula, reopen it, and verify that the test note was saved.
8. If the interface is erased, capture before/after screenshots and
   provide Windows version/SP1, DPI scale, display driver and step.
9. Run SMOKE-PDF-WIN7.cmd. It must create pdf-smoke-output.pdf with
   no errors. Open the output in a PDF reader and check that text exists.
10. Test both executables on Windows 10/11 when available.

The launcher uses a test vault and separate data directory beside this file.
Do not use private files, network sync, or normal automatic updating.
Do not overwrite your production Akula installation.

Shiva requires an explicit small directory of valid fonts (SHIVA_FONT_DIR).
The launchers prepare it by copying Arial from the local Windows installation.
No fonts are redistributed; if the font is missing they stop with a clear error.

The two experimental EXEs were built from independent pinned sources using
the same Windows 7 target. The Shiva PDF converter belongs next to Akula.
Both were PE-audited; Shiva passed a text-to-PDF test on Windows Server 2022,
which is NOT proof of a native Windows 7 runtime test.

The experimental Akula EXE was compiled with the Win7 target, and the import
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
set "SHIVA_FONT_DIR=%ROOT%shiva-fonts"
call "%ROOT%PREPARE-SHIVA-FONTS.cmd"
if errorlevel 1 exit /b 1
"%ROOT%akula-app.exe" --vault "%ROOT%vault" --data-dir "%ROOT%state"
"""

PDF_SMOKE = """@echo off
setlocal
set "ROOT=%~dp0"
if not exist "%ROOT%shiva-pdf-preview.exe" (echo ERROR: Shiva EXE missing & exit /b 1)
set "SHIVA_FONT_DIR=%ROOT%shiva-fonts"
call "%ROOT%PREPARE-SHIVA-FONTS.cmd"
if errorlevel 1 exit /b 1
if exist "%ROOT%pdf-smoke-output.pdf" del /q "%ROOT%pdf-smoke-output.pdf"
"%ROOT%shiva-pdf-preview.exe" "%ROOT%pdf-smoke-input.txt" "%ROOT%pdf-smoke-output.pdf"
if errorlevel 1 (echo ERROR: PDF conversion failed & exit /b 1)
if not exist "%ROOT%pdf-smoke-output.pdf" (echo ERROR: PDF output missing & exit /b 1)
for %%F in ("%ROOT%pdf-smoke-output.pdf") do if %%~zF LSS 64 (echo ERROR: output too small & exit /b 1)
echo PDF conversion completed: pdf-smoke-output.pdf
exit /b 0
"""

FONT_PREP = r"""@echo off
rem Only copy a font supplied by this Windows installation. The lab ZIP
rem carries no third-party fonts and does not require network access.
if not defined SHIVA_FONT_DIR (echo ERROR: SHIVA_FONT_DIR not set & exit /b 1)
if not exist "%SHIVA_FONT_DIR%\" mkdir "%SHIVA_FONT_DIR%" || exit /b 1
if not exist "%SHIVA_FONT_DIR%\arial.ttf" (
    if not exist "%WINDIR%\Fonts\arial.ttf" (
        echo ERROR: Arial not found in the Windows font directory.
        exit /b 1
    )
    copy /b /y "%WINDIR%\Fonts\arial.ttf" "%SHIVA_FONT_DIR%\arial.ttf" >nul
    if errorlevel 1 (echo ERROR: Could not provision PDF font & exit /b 1)
)
if not exist "%SHIVA_FONT_DIR%\arial.ttf" (echo ERROR: PDF font unavailable & exit /b 1)
exit /b 0
"""

PDF_SMOKE_INPUT = "Akula Windows 7 PDF companion sample.\r\nA test of Shiva text-to-PDF.\r\n"


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--exe", type=Path, required=True)
    p.add_argument("--audit", type=Path, required=True)
    p.add_argument("--sidecar", type=Path, required=True)
    p.add_argument("--sidecar-audit", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--revision", required=True)
    args = p.parse_args()
    report = json.loads(args.audit.read_text(encoding="utf-8"))
    expected = {"CoCreateFreeThreadedMarshaler", "CoTaskMemFree"}
    if set(report.get("imports", {}).get("combase.dll", [])) != expected:
        fail("unexpected combase.dll exports: cannot safely remap")
    if report.get("known_post_win7_imports"):
        fail("known Windows 8+ static imports remain")

    sidecar_report = json.loads(args.sidecar_audit.read_text(encoding="utf-8"))
    sidecar = args.sidecar.read_bytes()
    sidecar_digest = hashlib.sha256(sidecar).hexdigest()
    if sidecar[:2] != b"MZ" or len(sidecar) < 512 or u16(sidecar, u32(sidecar, 0x3C) + 4) != 0x8664:
        fail("Shiva PDF sidecar is not an x64 PE")
    if sidecar_report.get("known_post_win7_imports"):
        fail("Shiva PDF sidecar has newer Windows API imports")
    if "combase.dll" in sidecar_report.get("potentially_risky_imported_dlls", []):
        fail("Shiva PDF sidecar has combase.dll dependency")
    if sidecar_report.get("binary_sha256", "").lower() != sidecar_digest:
        fail("Shiva PDF sidecar SHA does not match its PE audit")
    if sidecar_report.get("binary_bytes") != len(sidecar):
        fail("Shiva PDF sidecar size does not match its PE audit")
    if sidecar_report.get("imported_function_count", 0) < 1:
        fail("Shiva PDF sidecar PE import list is empty")

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
    (args.out / "shiva-pdf-preview.exe").write_bytes(sidecar)
    digest = hashlib.sha256(data).hexdigest()
    (args.out / "README-WIN7-LAB.txt").write_text(README, encoding="utf-8")
    (args.out / "START-WIN7-LAB.cmd").write_text(LAUNCHER, encoding="ascii")
    (args.out / "SMOKE-PDF-WIN7.cmd").write_text(PDF_SMOKE, encoding="ascii")
    (args.out / "PREPARE-SHIVA-FONTS.cmd").write_text(FONT_PREP, encoding="ascii")
    (args.out / "pdf-smoke-input.txt").write_text(PDF_SMOKE_INPUT, encoding="ascii")
    provenance = (
        "source_sha=" + args.revision + "\n"
        "patched_exe_sha256=" + digest + "\n"
        "patched_exe_bytes=" + str(len(data)) + "\n"
        "shiva_pdf_source_commit=40b72a90b1dac39812c77c7321e57d613a81cb51\n"
        "shiva_pdf_sha256=" + sidecar_digest + "\n"
        "shiva_pdf_bytes=" + str(len(sidecar)) + "\n"
        "shiva_pdf_pe_audit_no_known_win8_imports=true\n"
        "shiva_pdf_smoke_tested_on_windows2022=true\n"
        "pe_rewrite=combase.dll to ole32.dll for 2 verified Co* imports\n"
        "runtime_win7_verified=false\n"
        "production_release=false\n"
    )
    (args.out / "PROVENANCE.txt").write_text(provenance, encoding="ascii")
    archive = args.out / "Akula-Win7-LAB-portable.zip"
    with zipfile.ZipFile(archive, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
        for name in ("akula-app.exe", "shiva-pdf-preview.exe", "START-WIN7-LAB.cmd",
                     "SMOKE-PDF-WIN7.cmd", "PREPARE-SHIVA-FONTS.cmd",
                     "pdf-smoke-input.txt",
                     "README-WIN7-LAB.txt", "PROVENANCE.txt"):
            zf.write(args.out / name, arcname=name)
    with zipfile.ZipFile(archive) as zf:
        if zf.testzip() is not None or len(zf.namelist()) != 8:
            fail("portable archive integrity check failed")
        if hashlib.sha256(zf.read("shiva-pdf-preview.exe")).hexdigest() != sidecar_digest:
            fail("PDF sidecar changed in portable archive")
        if hashlib.sha256(zf.read("akula-app.exe")).hexdigest() != digest:
            fail("Akula changed in portable archive")
    print("WIN7_LAB_PORTABLE_READY", archive, digest, archive.stat().st_size)


if __name__ == "__main__":
    main()
