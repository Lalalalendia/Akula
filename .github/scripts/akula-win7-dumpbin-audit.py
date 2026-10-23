#!/usr/bin/env python3
"""Read-only audit of dumpbin /imports output for the Akula Windows 7 lab.

Static imports only, not dynamic loading or runtime compatibility.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re

DLL_NAME = re.compile(r"^\s{2,}([A-Za-z0-9_.+-]+\.(?:dll|exe))\s*$", re.IGNORECASE)
SYMBOL = re.compile(r"^\s+[0-9A-Fa-f]+\s+([A-Za-z_?@][A-Za-z0-9_?@$+.:-]*)\s*$")
POST_WIN7 = {
    "GetSystemTimePreciseAsFileTime": "Windows 8",
    "WaitOnAddress": "Windows 8",
    "WakeByAddressSingle": "Windows 8",
    "WakeByAddressAll": "Windows 8",
    "ProcessPrng": "Windows 8+ (client)",
    "RoOriginateErrorW": "Windows 8",
    "GetPointerType": "Windows 8",
    "CreateDXGIFactory2": "Windows 8.1",
}
RISK_DLLS = {
    "combase.dll",
    "api-ms-win-core-synch-l1-2-0.dll",
    "api-ms-win-core-winrt-error-l1-1-0.dll",
    "bcryptprimitives.dll",
}


def parse_dumpbin(text: str) -> dict[str, list[str]]:
    imports: dict[str, set[str]] = {}
    current_dll: str | None = None
    in_imports = False
    for line in text.splitlines():
        if "Section contains the following imports:" in line:
            in_imports = True
            current_dll = None
            continue
        if not in_imports:
            continue
        if re.match(r"^\s*Summary\s*$", line):
            break
        dll = DLL_NAME.fullmatch(line)
        if dll:
            current_dll = dll.group(1).lower()
            imports.setdefault(current_dll, set())
            continue
        symbol = SYMBOL.fullmatch(line)
        if current_dll and symbol:
            imports[current_dll].add(symbol.group(1))
    return {dll: sorted(symbols, key=str.casefold) for dll, symbols in sorted(imports.items())}


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--dumpbin", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--sha256", required=True)
    p.add_argument("--bytes", type=int, required=True)
    args = p.parse_args()
    imports = parse_dumpbin(args.dumpbin.read_text(encoding="utf-8-sig", errors="replace"))
    num_symbols = sum(len(v) for v in imports.values())
    if not imports or not num_symbols:
        raise SystemExit("FAIL_CLOSED: No import DLLs or symbols parsed")
    bad = [
        {"dll": dll, "symbol": symbol, "introduced": POST_WIN7[symbol]}
        for dll, symbols in imports.items()
        for symbol in symbols
        if symbol in POST_WIN7
    ]
    risky = sorted(set(imports) & RISK_DLLS)
    report = {
        "scope": "static PE imports only; Win7 not executed",
        "binary_sha256": args.sha256,
        "binary_bytes": args.bytes,
        "imported_dll_count": len(imports),
        "imported_function_count": num_symbols,
        "known_post_win7_imports": bad,
        "potentially_risky_imported_dlls": risky,
        "imports": imports,
        "runtime_tested": False,
    }
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "full-audit.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    lines = [
        "# Akula native Win7 PE static import audit",
        "- Binary SHA-256: " + args.sha256,
        "- Binary size: " + str(args.bytes),
        "- Imported DLLs: " + str(len(imports)),
        "- Imported functions: " + str(num_symbols),
        "- Only static imports; no Win7 execution has been tested.",
        "",
        "## Known post-Windows-7 imports",
    ]
    if bad:
        lines += ["- " + x["dll"] + "!" + x["symbol"] + " (" + x["introduced"] + ")" for x in bad]
    else:
        lines.append("- None from finite screening list. This is NOT proof of compatibility.")
    lines += ["", "## DLLs requiring closer inspection"]
    lines += ["- " + x for x in risky] if risky else ["- None from short list; inspect full JSON."]
    (args.out / "full-audit.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    print("\n".join(lines))


if __name__ == "__main__":
    main()
