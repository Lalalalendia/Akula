#!/usr/bin/env python3
"""Read-only, release-pinned PE import audit for the Akula Windows 7 lab.

The installer and extracted executables are NEVER launched.
This is a screening probe, not evidence of Windows 7 runtime compatibility.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

# Screening list only: other missing Win7 imports may exist.
POST_WIN7_APIS = {
    "GetSystemTimePreciseAsFileTime": "Windows 8",
    "GetSystemTimeAdjustmentPrecise": "Windows 8",
    "WaitOnAddress": "Windows 8",
    "WakeByAddressSingle": "Windows 8",
    "WakeByAddressAll": "Windows 8",
    "GetPointerInfo": "Windows 8",
    "GetPointerType": "Windows 8",
    "GetPointerTouchInfo": "Windows 8",
    "GetPointerPenInfo": "Windows 8",
    "GetDpiForMonitor": "Windows 8.1",
    "GetDpiForWindow": "Windows 10",
    "GetSystemMetricsForDpi": "Windows 10",
    "AdjustWindowRectExForDpi": "Windows 10",
    "SetThreadDescription": "Windows 10",
    "GetThreadDescription": "Windows 10",
    "SetProcessDpiAwarenessContext": "Windows 10",
    "EnableNonClientDpiScaling": "Windows 10",
    "RoOriginateErrorW": "Windows 8",
    "ProcessPrng": "Windows 8",
}

_DLL = re.compile(r"^\s*DLL Name:\s*(\S+)", re.IGNORECASE)
_SYMBOL = re.compile(r"^\s*[0-9a-fA-F]+\s+\d+\s+(\S+)")


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def parse_imports(text: str) -> list[dict[str, str]]:
    dll = ""
    symbols: list[dict[str, str]] = []
    for line in text.splitlines():
        match = _DLL.match(line)
        if match:
            dll = match.group(1)
            continue
        match = _SYMBOL.match(line)
        if dll and match:
            name = match.group(1)
            if name.startswith("0x") or name.lower() in {"none", "ordinal"}:
                continue
            symbols.append({"dll": dll, "symbol": name})
    return symbols


def run(root: Path, output: Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    raw = output / "raw"
    raw.mkdir(exist_ok=True)
    binaries = sorted(
        (p for p in root.rglob("*") if p.is_file() and p.suffix.lower() in {".exe", ".dll", ".pyd"}),
        key=lambda p: str(p).lower(),
    )
    main = [p for p in binaries if p.name.lower() == "akula-app.exe"]
    if not main:
        raise RuntimeError("NSIS extracted no akula-app.exe; no compatibility claim is possible")
    if len(main) != 1:
        raise RuntimeError(f"Ambiguous akula-app.exe candidates: {len(main)}")

    entries = []
    for i, path in enumerate(binaries):
        if path.open("rb").read(2) != b"MZ":
            continue
        proc = subprocess.run(
            ["x86_64-w64-mingw32-objdump", "-p", str(path)],
            text=True, capture_output=True, check=False, timeout=60,
        )
        if proc.returncode:
            if path == main[0]:
                raise RuntimeError(f"PE import parser failed for Akula executable: {proc.stderr[:500]}")
            continue
        dump = proc.stdout
        (raw / f"{i:03d}-{path.name}.objdump.txt").write_text(dump, encoding="utf-8")
        symbols = parse_imports(dump)
        # Also match raw symbol names in case a binutils output format differs.
        hits = [
            {"symbol": api, "minimum_windows": minver}
            for api, minver in POST_WIN7_APIS.items()
            if re.search(r"\b" + re.escape(api) + r"\b", dump, re.IGNORECASE)
        ]
        entries.append({
            "file": path.relative_to(root).as_posix(),
            "sha256": digest(path),
            "size": path.stat().st_size,
            "dlls": sorted({x["dll"] for x in symbols}, key=str.lower),
            "parsed_import_count": len(symbols),
            "imports": symbols,
            "known_post_win7_imports": hits,
            "pe_header_os_lines": [
                line.strip() for line in dump.splitlines()
                if "OperatingSystemVersion" in line or "SubsystemVersion" in line
            ],
        })

    target = next((x for x in entries if x["file"] == main[0].relative_to(root).as_posix()), None)
    if not target or not target["dlls"]:
        raise RuntimeError("Akula executable has no parsed import DLLs: examine objdump output")
    result = {
        "release": os.environ.get("AKULA_RELEASE_TAG"),
        "installer_expected_sha256": os.environ.get("AKULA_RELEASE_SHA256"),
        "main_executable": target["file"],
        "main_sha256": target["sha256"],
        "note": "Static imports only. Does not cover dynamic LoadLibrary/GetProcAddress calls or actual OS execution.",
        "files": entries,
    }
    (output / "report.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    lines = [
        "# Akula Windows 7 PE import audit",
        f"- Release: {result['release']}",
        f"- Pinned NSIS installer SHA-256: {result['installer_expected_sha256']}",
        f"- Main executable: {target['file']}",
        f"- Main executable SHA-256: {target['sha256']}",
        "- Method: checked GitHub Release SHA-256, extracted NSIS without running it, inspected PE import tables.",
        "- Limitation: no Win7 VM, only a finite list of modern APIs; other OS or dependency failures may remain.",
        "",
        "## Known post-Win7 API imports (screening list)",
    ]
    total = 0
    for item in entries:
        for match in item["known_post_win7_imports"]:
            lines.append(f"- `{item['file']}`: `{match['symbol']}` ({match['minimum_windows']}+)")
            total += 1
    if not total:
        lines.append("- None from the screening list. **This does not prove Windows 7 compatibility.**")
    lines += ["", "## Imported DLL names"]
    for item in entries:
        lines += [f"- `{item['file']}`: {', '.join(item['dlls']) or '(not parsed)'}"]
    lines += ["", "## PE version headers"]
    for item in entries:
        lines += [f"- `{item['file']}`: {', '.join(item['pe_header_os_lines']) or '(none)'}"]
    (output / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: akula-win7-import-audit.py EXTRACTED_NSIS_DIR OUTPUT_DIR")
    run(Path(sys.argv[1]), Path(sys.argv[2]))
