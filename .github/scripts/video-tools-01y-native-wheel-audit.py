#!/usr/bin/env python3
"""VIDEO-TOOLS-01y: inert byte comparison of locked Windows wheels and yt-dlp.exe.

A SHA match of a wheel member to a PyInstaller CArchive member establishes
those *bytes*, not the installed wheel as a whole, native transitive source,
license compliance, or permission to release a GPLv3+ combined executable.
"""
import hashlib
import importlib.util
import io
import json
import os
import re
import struct
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
import zipfile
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent

def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod

X = load("video_tools_01x", "video-tools-01x-sdist-procurement.py")
R = load("video_tools_01r", "video-tools-01r-pe-archive-inventory.py")
STARTER = X.STARTER
WHEEL_TARGETS = X.SOURCE_TARGETS
CANDIDATES_EXPECTED = {"brotli":1, "cffi":1, "curl-cffi":1,
                       "mutagen":1, "pycryptodomex":1, "websockets":2}
NATIVE_ENDINGS = (".dll", ".pyd")
MAX_DOWNLOAD = 28 * 1024 * 1024
MAX_UNPACKED_WHEEL = 250 * 1024 * 1024
MAX_SINGLE_NATIVE = 70 * 1024 * 1024
MAX_NATIVE_TOTAL = 300 * 1024 * 1024
OUTPUT_NAME = "video-tools-01y-native-wheel-evidence-HOLD.json"

class Hold(Exception):
    pass

def need(v, reason):
    if not v:
        raise Hold("VIDEO_TOOLS_01Y_HOLD: " + reason)

def sha256(data):
    return hashlib.sha256(data).hexdigest()

def wheel_candidates(meta, name, version, upstream_hashes):
    """Return all allowed *possible* normal CPython 3.14 Windows wheel pins."""
    need(meta["info"].get("version") == version and
         X.canonical(meta["info"].get("name", "")) == name,
         "PyPI release metadata is for another distribution")
    candidates = []
    for row in meta.get("urls", []):
        filename = row.get("filename", "")
        if not X.compatible_cpython314_win64(filename):
            continue
        digest = row.get("digests", {}).get("sha256", "")
        if digest not in upstream_hashes:
            continue
        need(isinstance(filename, str) and len(filename) < 180 and
             re.fullmatch(r"[A-Za-z0-9_.+-]+\.whl", filename) is not None,
             "unsafe candidate wheel filename")
        parsed = urllib.parse.urlparse(row.get("url", ""))
        need(parsed.scheme == "https" and parsed.hostname == "files.pythonhosted.org",
             "candidate wheel URL not from PyPI release host")
        need(row.get("packagetype") == "bdist_wheel",
             "candidate wheel has unexpected source type")
        candidates.append(row)
    need(len(candidates) == CANDIDATES_EXPECTED[name] and
         len({r["filename"] for r in candidates}) == len(candidates) and
         len({r["digests"]["sha256"] for r in candidates}) == len(candidates),
         "candidate wheel set differs from locked release inventory " + name)
    return sorted(candidates, key=lambda r: r["filename"])

def acquire_wheel_bytes(candidate):
    """Read only; do not write or execute downloaded wheel bytes."""
    request = urllib.request.Request(candidate["url"], headers={
        "User-Agent": "Akula-VIDEO-TOOLS-01y-NONRELEASE-provenance",
    })
    with urllib.request.urlopen(request, timeout=90) as res:
        need(urllib.parse.urlparse(res.geturl()).hostname == "files.pythonhosted.org",
             "wheel download redirected away from PyPI")
        payload = res.read(MAX_DOWNLOAD + 1)
    need(1024 < len(payload) <= MAX_DOWNLOAD and
         sha256(payload) == candidate["digests"]["sha256"],
         "wheel bytes fail upstream-locked SHA or bound")
    return payload

def safe_wheel_name(name):
    return X.safe_member(name) and not name.startswith("/") and "//" not in name

def native_member_name(name):
    return name.lower().endswith(NATIVE_ENDINGS)

def wheel_native_members(data, name, version):
    need(zipfile.is_zipfile(io.BytesIO(data)), "wheel is not a ZIP")
    results = []
    with zipfile.ZipFile(io.BytesIO(data), "r") as archive:
        infos = archive.infolist()
        need(1 < len(infos) <= 10000, "wheel entries outside safe bound")
        names = [z.filename for z in infos]
        need(len(names) == len(set(names)) and
             all(safe_wheel_name(k) for k in names),
             "duplicate, traversing or invalid wheel ZIP name")
        need(sum(x.file_size for x in infos) <= MAX_UNPACKED_WHEEL,
             "wheel exceeds uncompressed quota")
        for member in infos:
            need(member.file_size <= MAX_UNPACKED_WHEEL and
                 (member.external_attr >> 16) & 0o170000 != 0o120000,
                 "wheel contains oversized member or symlink")
        metadata = [z for z in infos if z.filename.endswith(".dist-info/METADATA")]
        need(len(metadata) == 1 and metadata[0].file_size < 1024 * 1024,
             "wheel requires exactly one bounded distribution METADATA")
        body = archive.read(metadata[0]).decode("utf-8")
        header = {}
        for line in body.splitlines():
            if line.startswith("Name: ") or line.startswith("Version: "):
                k, v = line.split(": ", 1)
                header[k] = v.strip()
        need(X.canonical(header.get("Name", "")) == name and
             header.get("Version") == version,
             "wheel internal metadata package/version drift")
        total_native = 0
        for member in infos:
            if member.is_dir() or not native_member_name(member.filename):
                continue
            need(member.file_size <= MAX_SINGLE_NATIVE,
                 "wheel native library exceeds per-file bound")
            raw = archive.read(member)
            need(len(raw) == member.file_size, "wheel native member size changed")
            total_native += len(raw)
            need(total_native <= MAX_NATIVE_TOTAL, "wheel native contents too large")
            results.append({
                "wheel_member": member.filename,
                "basename": member.filename.rsplit("/", 1)[-1].lower(),
                "bytes": len(raw),
                "sha256": sha256(raw),
            })
    return sorted(results, key=lambda x: x["wheel_member"])

def archive_native_members(image):
    """Read only PyInstaller CArchive BINARY rows and DLL/PYD names.

    Parse original TOC through 01r, then validate and inspect bounded member
    offsets independently before decompressing. No binary is run or uploaded.
    """
    parsed = R.parse_carchive(image)
    need(parsed["carchive_count"] == 154 and parsed["python_module_count"] == 1574,
         "actual pinned EXE CArchive/PYZ inventory drift")
    marker = image.rfind(R.MAGIC)
    need(marker >= 0, "pinned EXE archive marker missing")
    _, archive_size, toc_off, toc_size, _, _ = R.COOKIE.unpack_from(image, marker)
    start = marker + R.COOKIE.size - archive_size
    toc = image[start + toc_off:start + toc_off + toc_size]
    pos = 0
    out = []
    total = 0
    while pos < len(toc):
        length, offset, size, expanded, compressed, kind = R.ENTRY.unpack_from(toc, pos)
        need(R.ENTRY.size < length <= 4096 and pos + length <= len(toc),
             "native TOC entry bounds invalid")
        name = R.safe_name(toc[pos + R.ENTRY.size:pos + length].split(b"\0", 1)[0].decode("utf8"))
        kind = kind.decode("ascii")
        if kind in ("b", "x") and native_member_name(name):
            need(0 <= offset <= toc_off and 0 < size <= toc_off - offset and
                 0 < expanded <= MAX_SINGLE_NATIVE and
                 compressed in (0, 1), "native TOC payload extent invalid")
            raw = image[start + offset:start + offset + size]
            if compressed:
                unzip = zlib.decompressobj()
                payload = unzip.decompress(raw, expanded + 1)
                need(len(payload) <= expanded and
                     not unzip.unconsumed_tail, "native decompression bound exceeded")
                payload += unzip.flush(expanded + 1 - len(payload))
                need(unzip.eof and not unzip.unused_data and
                     len(payload) == expanded, "native decompressed bytes invalid")
            else:
                payload = raw
                need(len(payload) == expanded, "uncompressed native length changed")
            total += len(payload)
            need(total <= MAX_NATIVE_TOTAL, "entire native CArchive exceeds safe bound")
            out.append({"archive_member": name,
                        "basename": name.replace("\\", "/").rsplit("/", 1)[-1].lower(),
                        "bytes": len(payload), "sha256": sha256(payload),
                        "carchive_kind": kind})
        pos += length
    need(1 < len(out) < 155 and len({z["archive_member"] for z in out}) == len(out),
         "CArchive native component set invalid")
    return sorted(out, key=lambda x: x["archive_member"])

def match_wheel_to_archive(wheel_native, exe_native):
    matches = []
    mismatches = []
    no_name = []
    for file in wheel_native:
        exact = [x for x in exe_native if x["sha256"] == file["sha256"] and
                 x["bytes"] == file["bytes"]]
        same_name = [x for x in exe_native if x["basename"] == file["basename"]]
        if exact:
            state = "NATIVE_BYTES_EXACT_IN_EXE"
        elif same_name:
            state = "BASENAME_ONLY_NATIVE_BYTES_DIFFER"
        else:
            state = "NO_EXE_NATIVE_BASENAME_OR_SHA_MATCH"
        row = {
            "wheel_member": file["wheel_member"],
            "wheel_sha256": file["sha256"],
            "wheel_size": file["bytes"],
            "evidence_class": state,
            "exact_native_archive_members": sorted(x["archive_member"] for x in exact),
            "same_basename_archive_members": sorted(x["archive_member"] for x in same_name),
        }
        if exact:
            matches.append(row)
        elif same_name:
            mismatches.append(row)
        else:
            no_name.append(row)
    return {
        "native_wheel_member_count": len(wheel_native),
        "byte_matches": matches,
        "same_name_different_bytes": mismatches,
        "not_seen_in_native_archive": no_name,
        "exact_native_members_in_exe_count": len(matches),
        "native_wheel_member_set_fully_found_in_exe":
            len(wheel_native) > 0 and len(matches) == len(wheel_native),
        "full_wheel_installed_proven": False,
        "native_corresponding_source_verified": False,
    }

def audit(allow_path, ytroot, exe_path, output_path):
    head = os.environ.get("EXPECTED_HEAD_SHA", "")
    need(os.environ.get("GITHUB_ACTIONS") == "true" and
         os.environ.get("GITHUB_EVENT_NAME") == "pull_request" and
         re.fullmatch(r"[0-9a-f]{40}", head) is not None,
         "only exact PR HEAD can produce native evidence")
    akula = Path(os.environ.get("GITHUB_WORKSPACE", "")).resolve(strict=True)
    tmp = Path(os.environ.get("RUNNER_TEMP", "")).resolve(strict=True)
    ytroot = Path(ytroot).resolve(strict=True)
    need(STARTER.git_head(akula) == head and
         STARTER.git_head(ytroot) == R.YT_COMMIT,
         "Akula or yt-dlp Git checkout differs from pin")
    allow = json.loads(Path(allow_path).read_text("utf8"))
    STARTER.check_release_hold(allow)
    exe = R.exact_runner_path(exe_path, tmp, "yt-dlp.exe")
    output = R.exact_runner_path(output_path, tmp, OUTPUT_NAME)
    need(exe.is_file() and exe.stat().st_size == R.YT_SIZE and
         not output.exists(), "EXE absent/size wrong or output already exists")
    image = exe.read_bytes()
    need(sha256(image) == R.YT_SHA, "immutable combined EXE SHA-256 differs")
    native = archive_native_members(image)
    groups, hashes = X.parse_all_locks(ytroot)
    need(set(WHEEL_TARGETS) == set(CANDIDATES_EXPECTED) and
         set(groups["curl-cffi"]) >= set(WHEEL_TARGETS),
         "source-to-wheel target set drift")
    evidence = []
    total_wheels = 0
    for name, version in sorted(WHEEL_TARGETS.items()):
        meta = X.read_pypi_json(name, version)
        candidates = wheel_candidates(meta, name, version, hashes["curl-cffi"][name])
        rows = []
        for candidate in candidates:
            wheel_bytes = acquire_wheel_bytes(candidate)
            wheel_native = wheel_native_members(wheel_bytes, name, version)
            comparison = match_wheel_to_archive(wheel_native, native)
            rows.append({
                "wheel_filename": candidate["filename"],
                "downloaded_wheel_sha256_verified_in_upstream_lock":
                    candidate["digests"]["sha256"],
                "wheel_bytes": len(wheel_bytes),
                "wheel_bytes_uploaded_or_executed": False,
                "candidate_selected_by_historical_pip_log": False,
                "candidate_is_relevant_to_exact_frozen_exe": False,
                "native_comparison": comparison,
            })
            total_wheels += 1
        evidence.append({"distribution": name, "pinned_version": version,
                         "candidate_wheel_count": len(rows),
                         "wheel_candidates": rows})
    need(total_wheels == 7 and len(evidence) == 6,
         "upstream lock Windows wheel candidate count changed")
    report = {
        "schema": 1,
        "stage": "VIDEO-TOOLS-01y native wheel/CArchive SHA overlap / NONRELEASE",
        "exact_akula_head": head,
        "yt_dlp_exe_sha256_verified": R.YT_SHA,
        "yt_dlp_source_git_commit_verified": R.YT_COMMIT,
        "upstream_lock_build_records_total": sum(len(g) for g in groups.values()),
        "compatible_locked_wheel_candidate_count": total_wheels,
        "candidate_wheel_distributions": evidence,
        "embedded_native_archive_members": native,
        "embedded_native_archive_member_count": len(native),
        "native_members_extracted_to_disk_or_executed": False,
        "complete_exe_native_sbom": False,
        "historical_pip_wheel_install_selection_verified": False,
        "native_wheel_to_published_sdist_source_equivalence_verified": False,
        "external_curl_ssl_crypto_native_sources_verified": False,
        "cpython_314_embedding_source_verified": False,
        "notices_distribution_approved": False,
        "legal_approval": False,
        "release_admitted": False,
        "complete_corresponding_source_available": False,
        "corresponding_source_rebuilt_and_verified": False,
        "publisher_signing_authorized": False,
        "binary_sbom_complete": False,
        "unresolved": [
            "Native byte matches do not prove complete original wheel installation or its derivation from the pinned sdist",
            "Only six wheel distribution families tested; EXE contains other Python and native constituents",
            "PyInstaller may transform binaries or bundle system/redistributable DLLs; basename-only is never byte identity",
            "Which of the two websockets wheel variants was selected must be proven independently",
            "curl-cffi / libcurl, SSL, impersonation and CPython native build inputs remain unverified",
            "PyInstaller custom bootloader already failed bit-for-bit parity in 01w",
            "Remaining GPLv3+/AGPL corresponding source and exact notices delivery require independent approval",
        ],
    }
    with output.open("x", encoding="utf8") as fp:
        json.dump(report, fp, sort_keys=True, indent=2)
        fp.write("\n")
    hits = sum(len(w["native_comparison"]["byte_matches"])
               for group in evidence for w in group["wheel_candidates"])
    print("VIDEO_TOOLS_01Y_NATIVE_WHEEL_AUDIT_PASS wheels=" + str(total_wheels) +
          " exe_native=" + str(len(native)) +
          " exact_native_overlaps=" + str(hits) +
          " complete_source=0 legal_approval=0 release=0")

if __name__ == "__main__":
    try:
        need(len(sys.argv) == 5,
             "usage: 01y-wheel.py allowlist.json pinned-yt-root exact-EXE report.json")
        audit(Path(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4])
    except (Hold, X.Hold, R.Hold, STARTER.Hold, KeyError, OSError, ValueError,
            TypeError, UnicodeError, struct.error, zlib.error, zipfile.BadZipFile,
            urllib.error.URLError, subprocess.SubprocessError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
