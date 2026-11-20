#!/usr/bin/env python3
"""VIDEO-TOOLS-01r: read pinned yt-dlp.exe PyInstaller TOC without executing it.

This is a deliberately bounded, standard-library-only CArchive/PYZ indexer.
It produces file and Python-module *names*, not a complete linked-binary SBOM,
license grant, corresponding-source validation, or release permission.
Layout: https://pyinstaller.org/en/stable/advanced-topics.html
"""
import hashlib
import io
import json
import marshal
import os
import re
import struct
import subprocess
import sys
from pathlib import Path

MAGIC = b"MEI\x0c\x0b\x0a\x0b\x0e"
COOKIE = struct.Struct(">8sIIII64s")
ENTRY = struct.Struct(">IIIIBc")
EXE_MAX = 64 * 1024 * 1024
TOC_MAX = 8 * 1024 * 1024
NAME_MAX = 1024
ITEM_MAX = 30000
YT_SHA = "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d"
YT_SIZE = 23575744
YT_TAG = "2026.10.26"
YT_COMMIT = "f264a6cbd1f630894da0f368df00f8aae50140e0"


class Hold(Exception):
    pass


def require(ok, reason):
    if not ok:
        raise Hold("VIDEO_TOOLS_01R_HOLD: " + reason)


def safe_name(name):
    require(isinstance(name, str) and 0 < len(name) <= NAME_MAX, "invalid TOC name")
    require("\x00" not in name and not name.startswith(("/", "\\")), "unsafe absolute TOC name")
    require(not re.match(r"^[A-Za-z]:", name), "unsafe drive-qualified TOC name")
    require(".." not in name.replace("\\", "/").split("/"), "unsafe traversing TOC name")
    return name


def parse_pyz(payload):
    require(len(payload) >= 14 and payload[:4] == b"PYZ\x00", "embedded PYZ magic absent")
    offset = struct.unpack(">i", payload[8:12])[0]
    require(12 <= offset < len(payload), "embedded PYZ TOC offset invalid")
    require(len(payload) - offset <= TOC_MAX, "embedded PYZ TOC unbounded")
    try:
        entries = dict(marshal.load(io.BytesIO(payload[offset:])))
    except (ValueError, TypeError, EOFError) as exc:
        raise Hold("VIDEO_TOOLS_01R_HOLD: embedded PYZ table unreadable") from exc
    require(0 < len(entries) <= ITEM_MAX, "embedded PYZ table missing or huge")
    indexed = []
    for name, data in entries.items():
        safe_name(name)
        require(isinstance(data, (tuple, list)) and len(data) == 3, "invalid PYZ module tuple")
        kind, pos, size = data
        require(isinstance(kind, int) and kind in (0, 1, 2, 3), "unexpected PYZ member kind")
        require(isinstance(pos, int) and isinstance(size, int), "invalid PYZ member offsets")
        require(12 <= pos < offset and 0 < size <= offset - pos, "PYZ member out of bounds")
        indexed.append({"name": name, "kind": kind, "compressed_size": size})
    return sorted(indexed, key=lambda x: x["name"])


def parse_carchive(image):
    require(isinstance(image, bytes) and 1024 <= len(image) <= EXE_MAX, "EXE is not bounded bytes")
    require(image[:2] == b"MZ", "Windows MZ header absent")
    marker = image.rfind(MAGIC)
    require(marker >= 0 and marker + COOKIE.size <= len(image), "PyInstaller CArchive cookie absent")
    magic, archive_size, toc_offset, toc_size, pyver, libname = COOKIE.unpack_from(image, marker)
    require(magic == MAGIC and 0 < pyver < 100000 and libname.split(b"\0")[0], "invalid CArchive cookie")
    end = marker + COOKIE.size
    start = end - archive_size
    require(64 <= start < marker and archive_size <= EXE_MAX, "invalid CArchive extent")
    require(0 < toc_size <= TOC_MAX, "CArchive TOC absent or huge")
    require(0 <= toc_offset and toc_offset + toc_size <= marker - start, "CArchive TOC outside payload")
    toc = image[start + toc_offset:start + toc_offset + toc_size]
    items = []
    pyz = []
    names = set()
    pos = 0
    while pos < len(toc):
        require(len(toc) - pos >= ENTRY.size, "truncated CArchive TOC entry")
        entry_len, file_offset, size, expanded, compressed, typecode = ENTRY.unpack_from(toc, pos)
        require(ENTRY.size < entry_len <= 4096 and pos + entry_len <= len(toc), "bad CArchive entry length")
        raw = toc[pos + ENTRY.size:pos + entry_len].split(b"\0", 1)[0]
        try:
            name = safe_name(raw.decode("utf-8"))
            kind = typecode.decode("ascii")
        except UnicodeError as exc:
            raise Hold("VIDEO_TOOLS_01R_HOLD: invalid CArchive item encoding") from exc
        require(kind in "bdzZMmsxol", "unknown CArchive member type")
        require(compressed in (0, 1), "unsupported CArchive compression")
        require(0 <= file_offset <= toc_offset and size <= toc_offset - file_offset, "CArchive member out of bounds")
        require(expanded <= EXE_MAX * 4, "CArchive member expanded-size unreasonable")
        if kind != "o":
            require(name not in names, "duplicate CArchive member")
            names.add(name)
        items.append({
            "name": name, "kind": kind, "packed_size": size,
            "unpacked_size": expanded, "compressed": bool(compressed)
        })
        if kind == "z":
            require(size > 12, "empty embedded PYZ")
            payload = image[start + file_offset:start + file_offset + size]
            if compressed:
                import zlib
                try:
                    payload = zlib.decompress(payload)
                except zlib.error as exc:
                    raise Hold("VIDEO_TOOLS_01R_HOLD: compressed PYZ invalid") from exc
            pyz.extend(parse_pyz(payload))
        pos += entry_len
        require(len(items) <= ITEM_MAX, "too many CArchive members")
    require(len(items) > 0 and len(pyz) > 0, "no recursive Python module inventory")
    return {
        "carchive_items": sorted(items, key=lambda x: (x["name"], x["kind"])),
        "embedded_python_items": sorted(pyz, key=lambda x: x["name"]),
        "carchive_count": len(items),
        "python_module_count": len(pyz),
        "python_version_code": pyver
    }


def make_report(allow, image):
    require(allow.get("schema") == 1 and allow.get("release_admitted") is False,
            "allowlist release gate changed")
    require(allow.get("installer_signing_authorized") is False and
            allow.get("stage_binary_distribution_authorized") is False and
            allow.get("independent_owner_approval_required") is True,
            "release-signing or distribution authorization changed")
    matches = [c for c in allow.get("components", []) if c.get("name") == "akula-yt-dlp.exe"]
    require(len(matches) == 1, "yt-dlp is not an exact unique allowlisted PE")
    yt = matches[0]
    require(yt.get("license") == "GPL-3.0-or-later" and
            yt.get("source_license") == "Unlicense" and
            yt.get("license_status") == "REVIEW_BUNDLED_DEPENDENCIES",
            "binary-vs-source license distinction changed")
    require(yt.get("reference") == YT_TAG and yt.get("tag_commit_sha") == YT_COMMIT,
            "pinned upstream source commit changed")
    require(yt.get("sha256") == YT_SHA and yt.get("size_bytes") == YT_SIZE,
            "pinned release digest or byte length changed")
    require(len(image) == YT_SIZE and hashlib.sha256(image).hexdigest() == YT_SHA,
            "actual yt-dlp release executable SHA-256 mismatch")
    contents = parse_carchive(image)
    return {
        "schema": 1,
        "stage": "VIDEO-TOOLS-01r NONRELEASE actual embedded archive-name review",
        "release_admitted": False,
        "legal_approval": False,
        "binary_constituent_license_review_complete": False,
        "complete_corresponding_source_available": False,
        "quickjs_mit_notice_packaged": False,
        "rust_mux_transitive_notices_complete": False,
        "publisher_signing_authorized": False,
        "artifact_kind": "names and TOC metadata only; executable not uploaded, never run",
        "limits": [
            "CArchive and embedded PYZ names are not package versions or a complete binary SBOM",
            "Native/DLL dynamic dependencies and external code require independent inspection",
            "Licensing of each embedded module requires mapping names to actual corresponding source",
            "The source for this exact combined Windows executable has not been preserved or approved"
        ],
        "binary": {
            "name": "akula-yt-dlp.exe",
            "upstream_release_asset": "yt-dlp.exe",
            "tag": YT_TAG,
            "source_commit": YT_COMMIT,
            "sha256": YT_SHA,
            "size_bytes": YT_SIZE,
            "combined_binary_license": "GPL-3.0-or-later",
            "source_repository_license": "Unlicense"
        },
        "inventory": contents
    }


def exact_runner_path(path, runner_temp, expected_name):
    temp = Path(runner_temp).resolve(strict=True)
    target = Path(path).resolve(strict=False)
    require(target.parent == temp and target.name == expected_name, "runner temp filename not exact")
    require(not target.is_symlink(), "symlink in runner proof input/output")
    return target


def main(argv):
    require(len(argv) == 4, "usage: program allowlist yt-dlp.exe report.json")
    require(os.environ.get("GITHUB_ACTIONS") == "true" and
            os.environ.get("GITHUB_EVENT_NAME") == "pull_request",
            "PR-only hosted nonrelease evidence")
    head = os.environ.get("EXPECTED_HEAD_SHA", "")
    require(bool(re.fullmatch(r"[0-9a-f]{40}", head)), "missing exact PR head")
    checkout = os.environ.get("GITHUB_WORKSPACE", "")
    actual_head = subprocess.check_output(["git", "-C", checkout, "rev-parse", "HEAD"],
                                          text=True, timeout=10).strip()
    require(actual_head == head, "checkout does not match PR HEAD")
    temp = os.environ.get("RUNNER_TEMP", "")
    exe = exact_runner_path(argv[2], temp, "yt-dlp.exe")
    output = exact_runner_path(argv[3], temp, "video-tools-01r-embedded-names-HOLD.json")
    require(exe.is_file() and not output.exists(), "input EXE absent or report already exists")
    require(exe.stat().st_size <= EXE_MAX, "input EXE exceeds bound")
    allow = json.loads(Path(argv[1]).read_text("utf8"))
    report = make_report(allow, exe.read_bytes())
    report["exact_akula_head"] = head
    with output.open("x", encoding="utf8", newline="\n") as fp:
        json.dump(report, fp, indent=2, ensure_ascii=False)
        fp.write("\n")
    print("VIDEO_TOOLS_01R_REAL_PYINSTALLER_INVENTORY_PASS "
          + "sha256=1 carchive_items=" + str(report["inventory"]["carchive_count"])
          + " python_modules=" + str(report["inventory"]["python_module_count"])
          + " binary_executed=0 complete_sbom=0 legal_approval=0 release=0")


if __name__ == "__main__":
    try:
        main(sys.argv)
    except (Hold, OSError, subprocess.SubprocessError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
