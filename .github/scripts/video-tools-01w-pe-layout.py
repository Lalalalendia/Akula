#!/usr/bin/env python3
"""01w: source/wheel PE layout forensics. Treat downloaded EXEs as inert bytes.

This is a NONRELEASE comparison. Nothing here proves complete corresponding
source, executable license compliance, or a historically reproducible build.
"""
import hashlib
import importlib.util
import json
import struct
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "video_tools_01v", HERE / "video-tools-01v-wheel-provenance.py")
V = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(V)

MAX_PE_BYTES = 12 * 1024 * 1024
BOOTLOADER_NAMES = ("run.exe", "run_d.exe", "runw.exe", "runw_d.exe")
MEMBER_PREFIX = "PyInstaller/bootloader/Windows-64bit-intel/"

class Hold(Exception):
    pass

def need(ok, why):
    if not ok:
        raise Hold("VIDEO_TOOLS_01W_HOLD: " + why)

def sha256(data):
    return hashlib.sha256(data).hexdigest()

def uint16(data, offset):
    return struct.unpack_from("<H", data, offset)[0]

def uint32(data, offset):
    return struct.unpack_from("<I", data, offset)[0]

def parse_pe(data):
    """Bounded static PE32+ inspection. No disassembly, imports or PE execution."""
    need(256 <= len(data) <= MAX_PE_BYTES and data[:2] == b"MZ",
         "PE file not bounded DOS MZ")
    pe_at = uint32(data, 0x3c)
    need(0x40 <= pe_at <= len(data) - 24 and
         data[pe_at:pe_at + 4] == b"PE\x00\x00", "PE signature invalid")
    coff = pe_at + 4
    machine, nsects, stamp, _, _, opt_size, characteristics = struct.unpack_from(
        "<HHIIIHH", data, coff)
    opt = coff + 20
    need(machine == 0x8664 and 0 < nsects <= 32 and 112 <= opt_size <= 1024
         and opt + opt_size + nsects * 40 <= len(data),
         "not valid bounded PE32+ x64")
    need(uint16(data, opt) == 0x20b, "not PE32+ optional header")
    checksum_at = opt + 64
    checksum = uint32(data, checksum_at)
    section_at = opt + opt_size
    sections = []
    for i in range(nsects):
        pos = section_at + i * 40
        (rawname, virtual_size, virtual_addr, raw_size, raw_ptr,
         _, _, _, _, flags) = struct.unpack_from("<8sIIIIIIHHI", data, pos)
        name_bytes = rawname.split(b"\x00", 1)[0]
        need(name_bytes and all(32 <= b < 127 for b in name_bytes),
             "unprintable PE section name")
        name = name_bytes.decode("ascii")
        need(raw_ptr <= len(data) and raw_size <= len(data) - raw_ptr,
             "PE section raw bounds invalid")
        content = data[raw_ptr:raw_ptr + raw_size]
        sections.append({
            "name": name,
            "virtual_size": virtual_size,
            "virtual_address": virtual_addr,
            "raw_size": raw_size,
            "raw_offset": raw_ptr,
            "characteristics": flags,
            "sha256": sha256(content),
        })
    normalized = bytearray(data)
    normalized[coff + 4:coff + 8] = b"\x00" * 4
    normalized[checksum_at:checksum_at + 4] = b"\x00" * 4
    return {
        "size": len(data),
        "sha256": sha256(data),
        "sha256_after_timestamp_checksum_mask": sha256(normalized),
        "pe_offset": pe_at,
        "machine": machine,
        "coff_timestamp": stamp,
        "characteristics": characteristics,
        "linker_major": data[opt + 2],
        "linker_minor": data[opt + 3],
        "size_of_image": uint32(data, opt + 56),
        "size_of_headers": uint32(data, opt + 60),
        "pe_checksum": checksum,
        "subsystem": uint16(data, opt + 68),
        "dll_characteristics": uint16(data, opt + 70),
        "sections": sections,
    }

def compare_pe(source_wheel, rebuilt):
    wheel = parse_pe(source_wheel)
    new = parse_pe(rebuilt)
    old_sections = wheel["sections"]
    new_sections = new["sections"]
    same_layout = [
        (x["name"], x["raw_size"], x["raw_offset"], x["virtual_size"],
         x["virtual_address"], x["characteristics"])
        for x in old_sections
    ] == [
        (x["name"], x["raw_size"], x["raw_offset"], x["virtual_size"],
         x["virtual_address"], x["characteristics"])
        for x in new_sections
    ]
    section_rows = []
    for name in sorted({x["name"] for x in old_sections + new_sections}):
        old = [x for x in old_sections if x["name"] == name]
        newer = [x for x in new_sections if x["name"] == name]
        need(len(old) <= 1 and len(newer) <= 1, "duplicate PE section name")
        section_rows.append({
            "section": name,
            "wheel_size": old[0]["raw_size"] if old else None,
            "rebuilt_size": newer[0]["raw_size"] if newer else None,
            "wheel_sha256": old[0]["sha256"] if old else None,
            "rebuilt_sha256": newer[0]["sha256"] if newer else None,
            "raw_bytes_identical": bool(old and newer and
                                        old[0]["sha256"] == newer[0]["sha256"]
                                        and old[0]["raw_size"] == newer[0]["raw_size"]),
        })
    matched = sum(row["raw_bytes_identical"] for row in section_rows)
    count = min(len(source_wheel), len(rebuilt))
    first_diff = next((i for i in range(count)
                       if source_wheel[i] != rebuilt[i]), None)
    if first_diff is None and len(source_wheel) != len(rebuilt):
        first_diff = count
    bytes_identical = source_wheel == rebuilt
    masked = wheel["sha256_after_timestamp_checksum_mask"] == (
        new["sha256_after_timestamp_checksum_mask"])
    if bytes_identical:
        class_name = "BYTE_IDENTICAL"
    elif masked:
        class_name = "ONLY_COFF_TIMESTAMP_AND_CHECKSUM_DIFFER"
    elif matched == len(section_rows) and same_layout:
        class_name = "SECTIONS_SAME_OTHER_BYTES_DIFFER"
    else:
        class_name = "SECTION_CONTENT_OR_LAYOUT_DIFFERS"
    return {
        "wheel": wheel,
        "rebuilt": new,
        "byte_identical": bytes_identical,
        "only_timestamp_checksum_may_differ": masked,
        "section_layout_identical": same_layout,
        "matching_raw_sections": matched,
        "total_distinct_section_names": len(section_rows),
        "section_comparisons": section_rows,
        "first_byte_difference_offset": first_diff,
        "difference_class": class_name,
    }

def observed_toolchain():
    commands = {
        "gcc": ["gcc", "--version"],
        "ld": ["ld", "--version"],
        "packages": ["pacman", "-Q", "mingw-w64-x86_64-gcc",
                     "mingw-w64-x86_64-binutils", "mingw-w64-x86_64-crt"],
    }
    results = {}
    for name, cmd in commands.items():
        result = subprocess.run(cmd, capture_output=True, text=True,
                                timeout=20, check=True)
        lines = [line.strip() for line in result.stdout.splitlines() if line.strip()]
        need(bool(lines) and len(result.stdout) < 10000,
             "missing bounded toolchain output " + name)
        results[name] = lines if name == "packages" else lines[0]
    return results

def run(argv):
    need(len(argv) == 5,
         "usage: script allowlist pyinstaller-source builder-source report.json")
    _, allowlist, root_arg, builder_arg, output_arg = argv
    head, temp = V.common_guard(allowlist)
    root = Path(root_arg)
    builder = Path(builder_arg)
    need(V.git_head(root) == V.PY_SHA and V.git_head(builder) == V.PYBUILD_SHA,
         "upstream source or builder SHA differs")
    need((root / "bootloader/wscript").is_file() and
         (builder / ".github/workflows/build.yml").is_file(),
         "source/recipe missing")
    wheel_file = V.require_temp(temp / V.PY_WHEEL_NAME, temp, V.PY_WHEEL_NAME)
    output = V.require_temp(output_arg, temp, "video-tools-01w-pe-layout-HOLD.json", False)
    need(V.file_sha(wheel_file) == V.PY_WHEEL_SHA, "wheel digest differs")
    with V.checked_zip(wheel_file) as z:
        V.metadata_match(z, "pyinstaller", "6.22.0")
        present = {k for k in z.namelist()
                   if k.startswith(MEMBER_PREFIX) and k.endswith(".exe")}
        need(present == {MEMBER_PREFIX + x for x in BOOTLOADER_NAMES},
             "unexpected wheel bootloader PE member set")
        results = []
        for name in BOOTLOADER_NAMES:
            path = MEMBER_PREFIX + name
            local = root / path
            need(local.is_file() and not local.is_symlink(),
                 "rebuilt PE missing: " + path)
            check = compare_pe(z.read(path), local.read_bytes())
            check["wheel_member"] = path
            results.append(check)
    report = V.base_report(head, "VIDEO-TOOLS-01w Win64 bootloader PE layout evidence")
    report.update({
        "source_git_commit_verified": V.PY_SHA,
        "builder_git_commit_verified": V.PYBUILD_SHA,
        "custom_wheel_sha256_verified": V.PY_WHEEL_SHA,
        "current_toolchain_observed": observed_toolchain(),
        "historical_runner_toolchain_attested": False,
        "upstream_release_date": "2026-08-19",
        "upstream_workflow_run": 32306212512,
        "upstream_build_job_logs_available": False,
        "images": results,
        "all_four_images_byte_identical": all(row["byte_identical"] for row in results),
        "all_four_only_timestamp_checksum_difference":
            all(row["only_timestamp_checksum_may_differ"] for row in results),
        "known_limitations": [
            "Original 2026-08-19 MSYS2 package versions cannot be proven from expired job logs",
            "The current runner has updated packages; version equality of GCC alone is insufficient",
            "PE section hashes and header fields are diagnostic, not a complete SBOM or source attestation",
            "No published EXE/wheel is executed or uploaded; artifact contains JSON metadata only",
            "This cannot approve corresponding source, GPL/AGPL notice compliance, or release",
        ],
    })
    V.save_report(report, output)
    differing = sum(not row["byte_identical"] for row in results)
    payloads = sum(row["difference_class"] == "SECTION_CONTENT_OR_LAYOUT_DIFFERS"
                   for row in results)
    print("VIDEO_TOOLS_01W_PE_LAYOUT_PASS images=4 differing=" +
          str(differing) + " section_or_layout_differences=" + str(payloads) +
          " complete_source=0 legal_approval=0 release=0")

if __name__ == "__main__":
    try:
        run(sys.argv)
    except (Hold, V.Hold, V.STARTER.Hold, OSError, ValueError, KeyError,
            IndexError, struct.error, subprocess.SubprocessError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
