#!/usr/bin/env python3
"""VIDEO-TOOLS-01s: compare a real PyInstaller-embedded notice to pinned source.

Evidence only: candidates never imply verified corresponding source, exact
dependency versions, license compatibility, or distribution permission.
"""
import collections
import hashlib
import importlib.util
import json
import os
import re
import struct
import subprocess
import sys
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "akula_video_tools_01r", HERE / "video-tools-01r-pe-archive-inventory.py")
R = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(R)

UPSTREAM = "f264a6cbd1f630894da0f368df00f8aae50140e0"
NOTICE_BLOB = "88f9872c66b456703e91ca32db4041e1ec47258d"
NOTICE_NAME = "THIRD_PARTY_LICENSES.txt"
MAX_NOTICE = 1024 * 1024

# A candidate mapping based only on a module root and a notice section title.
# The sections may describe all upstream platform bundles, not only this Win64 EXE.
CANDIDATE_TITLES = {
    "Cryptodome": "PyCryptodome",
    "websockets": "websockets",
    "mutagen": "mutagen",
    "urllib3": "urllib3",
    "curl_cffi": "curl_cffi",
    "requests": "requests",
    "cffi": "cffi",
    "charset_normalizer": "charset-normalizer",
    "idna": "idna",
    "pycparser": "pycparser",
    "certifi": "certifi",
    "brotli": "brotli",
}
# yt_dlp and yt_dlp_ejs have no direct third-party section title:
# the yt-dlp source Unlicense and bundled EJS provenance require separate review.
OWN_ROOTS = ("yt_dlp", "yt_dlp_ejs")
NATIVE_NOTICE_CANDIDATES = {
    "python314.dll": "Python",
    "python3.dll": "Python",
    "libssl-3.dll": "OpenSSL 3.0+",
    "libcrypto-3.dll": "OpenSSL 3.0+",
    "libffi-8.dll": "libffi",
    "sqlite3.dll": "SQLite",
}


def hold(why):
    raise R.Hold("VIDEO_TOOLS_01S_HOLD: " + why)


def git_blob_hash(data):
    return hashlib.sha1(b"blob " + str(len(data)).encode("ascii") + b"\0" + data).hexdigest()


def notice_sections(data):
    # Read headings only. Never copy the copyrighted body into a CI artifact.
    text = data.decode("utf-8")
    lines = text.splitlines()
    sections = []
    for i in range(1, len(lines)):
        if len(lines[i-1]) < 20 or not set(lines[i-1].strip()) <= {"-"}:
            continue
        m = re.fullmatch(r"([^|\n]{2,100}?)\s+\|\s+([^\n]{2,120})", lines[i].strip())
        if not m:
            continue
        title, license_hint = m.group(1).strip(), m.group(2).strip()
        if title and license_hint:
            sections.append({"component": title, "upstream_license_label": license_hint})
    return sections


def embedded_notice(image, entries):
    # Reuse verified CArchive layout, but extract ONLY the single non-code notice.
    marker = image.rfind(R.MAGIC)
    if marker < 0 or marker + R.COOKIE.size > len(image):
        hold("PyInstaller cookie missing")
    _, archive_length, toc_offset, toc_length, _, _ = R.COOKIE.unpack_from(image, marker)
    archive_start = marker + R.COOKIE.size - archive_length
    if archive_start < 0 or toc_length > R.TOC_MAX:
        hold("invalid verified archive extent")
    toc = image[archive_start + toc_offset:archive_start + toc_offset + toc_length]
    matches = [item for item in entries if item["name"] == NOTICE_NAME]
    if len(matches) != 1 or matches[0]["kind"] != "b":
        hold("actual bundled third-party notice not found once as data")
    offset = 0
    found = None
    while offset < len(toc):
        if len(toc) - offset < R.ENTRY.size:
            hold("truncated notice TOC")
        length, position, packed_len, unpacked_len, compressed, code = R.ENTRY.unpack_from(toc, offset)
        if length <= R.ENTRY.size or offset + length > len(toc):
            hold("invalid notice entry")
        name = toc[offset + R.ENTRY.size:offset + length].split(b"\0", 1)[0].decode("utf-8")
        if name == NOTICE_NAME:
            if found is not None or unpacked_len > MAX_NOTICE or packed_len > MAX_NOTICE:
                hold("duplicated or oversize embedded notice")
            if position + packed_len > toc_offset or compressed not in (0, 1) or code != b"b":
                hold("notice data extent or type invalid")
            packed = image[archive_start + position:archive_start + position + packed_len]
            if compressed:
                obj = zlib.decompressobj()
                payload = obj.decompress(packed, MAX_NOTICE + 1)
                if not obj.eof or obj.unused_data or obj.unconsumed_tail:
                    hold("truncated, trailing or expansion-bomb notice stream")
                payload += obj.flush()
            else:
                payload = packed
            if len(payload) != unpacked_len or len(payload) > MAX_NOTICE:
                hold("embedded notice size mismatch")
            found = payload
        offset += length
    if found is None:
        hold("embedded notice text missing")
    return found


def make_report(index, source, embedded):
    items = index["inventory"]["carchive_items"]
    modules = index["inventory"]["embedded_python_items"]
    if len(items) != 154 or len(modules) != 1574:
        hold("pinned actual binary inventory counts changed unexpectedly")
    roots = collections.Counter(item["name"].split(".", 1)[0] for item in modules)
    src_sections = notice_sections(source)
    embedded_sections = notice_sections(embedded)
    tagged = {item["component"]: item["upstream_license_label"] for item in src_sections}
    packed = {item["component"]: item["upstream_license_label"] for item in embedded_sections}
    family_rows = []
    for family in sorted(CANDIDATE_TITLES.keys() | set(OWN_ROOTS)):
        count = roots.pop(family, 0)
        if not count:
            continue
        title = CANDIDATE_TITLES.get(family)
        versions = sorted({
            m.group(1) for item in items
            if (m := re.fullmatch(
                re.escape(family) + r"-(\d[\w.]+)\.dist-info[\\/].+",
                item["name"], re.IGNORECASE))
        })
        family_rows.append({
            "python_root": family,
            "pyz_name_count": count,
            "embedded_dist_info_versions": versions,
            "version_evidence": "directory name only" if versions else "NONE",
            "candidate_notice_section": title,
            "present_in_tagged_notice": bool(title and title in tagged),
            "present_in_embedded_notice": bool(title and title in packed),
            "tagged_label": tagged.get(title) if title else None,
            "embedded_label": packed.get(title) if title else None,
            "review_status": "CANDIDATE_ONLY_NOT_A_LICENSE_DETERMINATION"
        })
    unknown_roots = [{"python_root": key, "module_name_count": count}
                     for key, count in sorted(roots.items())]
    native = [item for item in items if item["name"].lower().endswith((".dll", ".pyd", ".js"))]
    native_matches = [
        {
            "name": item["name"],
            "candidate_notice_section": NATIVE_NOTICE_CANDIDATES[item["name"]],
            "review_status": "CANDIDATE_ONLY_NOT_A_LICENSE_DETERMINATION",
        }
        for item in native if item["name"] in NATIVE_NOTICE_CANDIDATES
    ]
    covered = sum(row["pyz_name_count"] for row in family_rows)
    unresolved = sum(row["module_name_count"] for row in unknown_roots)
    if covered + unresolved != len(modules):
        hold("missing module root from review queue")
    src_unique = set(tagged)
    packed_unique = set(packed)
    return {
        "schema": 1,
        "stage": "VIDEO-TOOLS-01s NONRELEASE embedded binary notice and candidate provenance",
        "source_release_tag": "2026.10.26",
        "source_commit": UPSTREAM,
        "release_admitted": False,
        "legal_approval": False,
        "license_compatibility_review_complete": False,
        "complete_corresponding_source_available": False,
        "binary_constituent_license_review_complete": False,
        "notices_packaging_approved": False,
        "publisher_signing_authorized": False,
        "carchive_items": len(items),
        "pyz_module_names": len(modules),
        "candidate_group_module_names": covered,
        "unmapped_module_names": unresolved,
        "unmapped_roots": unknown_roots,
        "candidate_groups": family_rows,
        "native_candidate_matches": native_matches,
        "total_native_or_js_members": len(native),
        "upstream_notice": {
            "source_blob_sha1": NOTICE_BLOB,
            "tagged_bytes": len(source),
            "embedded_bytes": len(embedded),
            "size_delta_embedded_minus_tagged": len(embedded) - len(source),
            "byte_exact_match": embedded == source,
            "tagged_sha256": hashlib.sha256(source).hexdigest(),
            "embedded_sha256": hashlib.sha256(embedded).hexdigest(),
            "tagged_sections_count": len(src_sections),
            "embedded_sections_count": len(embedded_sections),
            "sections_only_in_tagged": sorted(src_unique - packed_unique),
            "sections_only_in_embedded": sorted(packed_unique - src_unique),
            "section_license_label_differences": [
                {"section": k, "tagged_label": tagged[k], "embedded_label": packed[k]}
                for k in sorted(src_unique & packed_unique) if tagged[k] != packed[k]
            ],
            "status": "BYTE_EXACT" if embedded == source else "DIFF_REQUIRES_MANUAL_REVIEW",
        },
        "known_limitations": [
            "Top-level Python module names are candidate families, not audited package ownership",
            "Version is asserted only from embedded .dist-info directory names",
            "Notice section presence is not evidence that code/license applies to Windows",
            "Missing notice section is not evidence of infringement",
            "Unmapped Python roots may include standard library; classification requires review",
            "PyInstaller packed notice may differ from tagged text; each discrepancy must be reconciled",
            "No complete linked-binary SBOM, license approval, source delivery or Windows signed release",
        ]
    }


def run(argv):
    if len(argv) != 6:
        hold("usage: script allowlist.exe-path index.json pinned_source_root report.json")
    _, allow_path, exe_path, index_path, source_root, out_path = argv
    head = os.environ.get("EXPECTED_HEAD_SHA", "")
    if (os.environ.get("GITHUB_ACTIONS") != "true" or
        os.environ.get("GITHUB_EVENT_NAME") != "pull_request" or
        not re.fullmatch(r"[0-9a-f]{40}", head)):
        hold("must run from exact pull_request HEAD on hosted runner")
    ws = os.environ.get("GITHUB_WORKSPACE", "")
    if subprocess.check_output(["git", "-C", ws, "rev-parse", "HEAD"],
                               text=True, timeout=10).strip() != head:
        hold("working tree not exact PR HEAD")
    if subprocess.check_output(["git", "-C", source_root, "rev-parse", "HEAD"],
                               text=True, timeout=10).strip() != UPSTREAM:
        hold("source checkout differs from pinned upstream commit")
    temp = os.environ.get("RUNNER_TEMP", "")
    exe = R.exact_runner_path(exe_path, temp, "yt-dlp.exe")
    idx = R.exact_runner_path(index_path, temp, "video-tools-01r-embedded-names-HOLD.json")
    output = R.exact_runner_path(out_path, temp, "video-tools-01s-notice-review-HOLD.json")
    if not exe.is_file() or not idx.is_file() or output.exists():
        hold("verified runner inputs missing or output exists")
    source_file = Path(source_root).resolve() / NOTICE_NAME
    if not source_file.is_file() or source_file.is_symlink():
        hold("tagged source notice missing or linked")
    source = source_file.read_bytes()
    if git_blob_hash(source) != NOTICE_BLOB:
        hold("tagged THIRD_PARTY_LICENSES.txt Git blob changed")
    index = json.loads(idx.read_text("utf8"))
    allow = json.loads(Path(allow_path).read_text("utf8"))
    image = exe.read_bytes()
    # 01r verifies binary and release allowlist again before matching entry metadata.
    baseline = R.make_report(allow, image)
    if index.get("exact_akula_head") != head or index.get("inventory") != baseline["inventory"]:
        hold("01r actual EXE index not on exact current HEAD")
    embed = embedded_notice(image, baseline["inventory"]["carchive_items"])
    report = make_report(baseline, source, embed)
    report["exact_akula_head"] = head
    with output.open("x", encoding="utf8", newline="\n") as fp:
        json.dump(report, fp, indent=2, ensure_ascii=False)
        fp.write("\n")
    n = report["upstream_notice"]
    print("VIDEO_TOOLS_01S_NOTICE_REVIEW_PASS "
          + f"actual_carchive={report['carchive_items']} "
          + f"actual_modules={report['pyz_module_names']} "
          + f"candidate_modules={report['candidate_group_module_names']} "
          + f"unmapped_modules={report['unmapped_module_names']} "
          + f"embedded_notice_bytes={n['embedded_bytes']} "
          + f"tagged_notice_bytes={n['tagged_bytes']} "
          + f"notice_equal={int(n['byte_exact_match'])} "
          + "complete_sbom=0 legal_approval=0 release=0")


if __name__ == "__main__":
    try:
        run(sys.argv)
    except (R.Hold, OSError, ValueError, KeyError, subprocess.SubprocessError,
            UnicodeError, zlib.error) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
