#!/usr/bin/env python3
"""VIDEO-TOOLS-01v: bounded, non-executing comparison of pinned wheel bytes.

Never run a downloaded wheel, DLL, or EXE. Reports are evidence, not a release
approval or a claim that a complete corresponding-source bundle exists.
"""
import hashlib
import importlib.util
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "video_tools_01t", HERE / "video-tools-01t-source-starter.py"
)
STARTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STARTER)

PY_SHA = "70fc17210920bce17f4ab09bbf8104b0dbd45338"
PYBUILD_SHA = "5eb86b862d61adc3df0f44e6e0b815b7d74fa3f5"
EJS_SHA = "4fb477f4af56880cfd324c48bd4294a2d2294e50"
PY_WHEEL_NAME = "pyinstaller-6.22.0-py3-none-win_amd64.whl"
PY_WHEEL_SHA = "294099ecb5fdd2a13ae4c29006d4e335b697a63b6a16cf052b90ec2b40bef05a"
PY_WHEEL_URL = ("https://github.com/yt-dlp/Pyinstaller-Builds/releases/download/"
                "2026.08.19.215425/" + PY_WHEEL_NAME)
EJS_WHEEL_NAME = "yt_dlp_ejs-0.8.0-py3-none-any.whl"
EJS_WHEEL_SHA = "79300e5fca7f937a1eeede11f0456862c1b41107ce1d726871e0207424f4bdb4"
EJS_SDIST_NAME = "yt_dlp_ejs-0.8.0.tar.gz"
EJS_SDIST_SHA = "d5fa1639f63b5c4af8d932495f60689d5370f1a095782c944f7f62a303eb104e"
EJS_METADATA_URL = "https://pypi.org/pypi/yt-dlp-ejs/0.8.0/json"
MAX_DOWNLOAD = 40 * 1024 * 1024
MAX_ARCHIVE_TOTAL = 300 * 1024 * 1024
EJS_SOURCE_PATHS = (
    "LICENSE", "hatch_build.py", "package.json", "package-lock.json",
    "pyproject.toml", "rollup.config.js", "src/yt/solver/main.ts",
    "src/yt/solver/lib.ts", "src/yt/solver/dynamic.lib.ts",
)
EJS_JS = {
    "yt_dlp_ejs/yt/solver/core.min.js": "dist/yt.solver.core.min.js",
    "yt_dlp_ejs/yt/solver/lib.min.js": "dist/yt.solver.lib.min.js",
}

class Hold(Exception):
    pass

def need(test, why):
    if not test:
        raise Hold("VIDEO_TOOLS_01V_HOLD: " + why)

def sha256(data):
    return hashlib.sha256(data).hexdigest()

def file_sha(path):
    return sha256(Path(path).read_bytes())

def git_head(root):
    return subprocess.check_output(
        ["git", "-C", str(root), "rev-parse", "HEAD"],
        text=True, timeout=15
    ).strip()

def safe_name(name):
    parts = name.rstrip("/").split("/")
    return (bool(name) and not name.startswith("/") and "\\" not in name and
            "\x00" not in name and all(x not in ("", ".", "..") for x in parts))

def checked_zip(path, maximum=MAX_ARCHIVE_TOTAL):
    need(zipfile.is_zipfile(path), "not a ZIP wheel")
    z = zipfile.ZipFile(path, "r")
    infos = z.infolist()
    names = [i.filename for i in infos]
    need(len(infos) <= 12000 and len(names) == len(set(names)),
         "ZIP duplicates or excess entries")
    need(all(safe_name(n) for n in names), "unsafe wheel ZIP path")
    need(sum(i.file_size for i in infos) <= maximum,
         "wheel exceeds uncompressed size limit")
    need(all(i.file_size <= maximum for i in infos),
         "individual wheel file exceeds limit")
    return z

def checked_sdist(path):
    need(tarfile.is_tarfile(path), "not a source tarball")
    tar = tarfile.open(path, "r:gz")
    members = tar.getmembers()
    need(len(members) <= 12000 and sum(x.size for x in members) <= MAX_ARCHIVE_TOTAL,
         "sdist unexpectedly large")
    need(len(members) == len(set(x.name for x in members)),
         "duplicate sdist names")
    need(all(safe_name(m.name) and (m.isfile() or m.isdir()) for m in members),
         "unsafe sdist entry")
    return tar

def metadata_match(zipf, expected_dist, expected_version):
    names = [n for n in zipf.namelist() if n.endswith(".dist-info/METADATA")]
    need(len(names) == 1, "wheel must have one METADATA")
    body = zipf.read(names[0]).decode("utf8")
    fields = {}
    for line in body.splitlines():
        if line.startswith(("Name: ", "Version: ")):
            key, val = line.split(": ", 1)
            fields[key] = val.strip()
    need(fields.get("Name", "").lower().replace("_", "-") == expected_dist and
         fields.get("Version") == expected_version, "wheel metadata name/version drift")
    return names[0]

def common_guard(allow_path):
    head = os.environ.get("EXPECTED_HEAD_SHA", "")
    need(os.environ.get("GITHUB_ACTIONS") == "true" and
         os.environ.get("GITHUB_EVENT_NAME") == "pull_request" and
         bool(re.fullmatch("[0-9a-f]{40}", head)),
         "evidence must run on exact PR HEAD only")
    workspace = Path(os.environ.get("GITHUB_WORKSPACE", "")).resolve(strict=True)
    temp = Path(os.environ.get("RUNNER_TEMP", "")).resolve(strict=True)
    need(git_head(workspace) == head, "Akula checkout != expected PR HEAD")
    allow = json.loads(Path(allow_path).read_text("utf8"))
    STARTER.check_release_hold(allow)
    return head, temp

def require_temp(path, temp, name, exists=True):
    p = Path(path).resolve(strict=exists)
    need(p == temp / name and (p.is_file() if exists else not p.exists()),
         "wheel/report not at exact isolated runner path " + name)
    return p

def get_url(url, target, digest, maxlen=MAX_DOWNLOAD):
    need(not target.exists(), "download target already exists")
    parsed = urllib.parse.urlparse(url)
    need(parsed.scheme == "https" and parsed.hostname in
         ("github.com", "files.pythonhosted.org"), "unexpected download host")
    req = urllib.request.Request(url, headers={"User-Agent": "Akula-01v-nonrelease-source-audit"})
    size = 0
    h = hashlib.sha256()
    try:
        with urllib.request.urlopen(req, timeout=90) as response, target.open("xb") as out:
            while True:
                chunk = response.read(65536)
                if not chunk:
                    break
                size += len(chunk)
                need(size <= maxlen, "download too large")
                h.update(chunk)
                out.write(chunk)
        need(size > 1024 and h.hexdigest() == digest, "downloaded artifact SHA-256 differs")
    except BaseException:
        target.unlink(missing_ok=True)
        raise
    return size

def download_ejs(temp):
    req = urllib.request.Request(EJS_METADATA_URL, headers={
        "User-Agent": "Akula-01v-nonrelease-source-audit"})
    with urllib.request.urlopen(req, timeout=40) as response:
        metadata = json.load(io.TextIOWrapper(response, encoding="utf8"))
    for filename, digest in ((EJS_WHEEL_NAME, EJS_WHEEL_SHA),
                             (EJS_SDIST_NAME, EJS_SDIST_SHA)):
        rows = [x for x in metadata.get("urls", []) if x.get("filename") == filename]
        need(len(rows) == 1 and rows[0].get("digests", {}).get("sha256") == digest,
             "pinned EJS PyPI metadata missing or digest differs: " + filename)
        get_url(rows[0]["url"], temp / filename, digest)
    return 2

def download_pyi(temp):
    return get_url(PY_WHEEL_URL, temp / PY_WHEEL_NAME, PY_WHEEL_SHA)

def base_report(head, component):
    return {
        "schema": 1, "stage": "VIDEO-TOOLS-01v NONRELEASE byte provenance",
        "exact_akula_head": head, "component": component,
        "release_admitted": False, "legal_approval": False,
        "complete_corresponding_source_available": False,
        "corresponding_source_rebuilt_and_verified": False,
        "publisher_signing_authorized": False,
        "windows_executables_included_in_report": False,
        "complete_license_notice_package": False,
        "binary_sbom_complete": False,
    }

def analyze_pyinstaller(wheel_path, source_root, builder_root, head):
    need(git_head(source_root) == PY_SHA and git_head(builder_root) == PYBUILD_SHA,
         "PyInstaller source or custom-builder commit drift")
    need((source_root / "bootloader/src/pyi_main.c").is_file() and
         (source_root / "bootloader/wscript").is_file() and
         (builder_root / ".github/workflows/build.yml").is_file(),
         "PyInstaller bootloader source/build recipe incomplete")
    need(file_sha(wheel_path) == PY_WHEEL_SHA, "custom PyInstaller wheel SHA mismatch")
    prefix = "PyInstaller/bootloader/Windows-64bit-intel/"
    with checked_zip(wheel_path) as wheel:
        meta = metadata_match(wheel, "pyinstaller", "6.22.0")
        entries = sorted(n for n in wheel.namelist()
                         if n.startswith(prefix) and n.endswith(".exe"))
        need(prefix + "run.exe" in entries and prefix + "runw.exe" in entries,
             "custom win_amd64 PyInstaller bootloader images not found")
        results = []
        for name in entries:
            data = wheel.read(name)
            built = source_root / "bootloader" / name[len("PyInstaller/bootloader/"):]
            have = built.is_file()
            rebuilt = built.read_bytes() if have else b""
            results.append({
                "wheel_member": name, "wheel_sha256": sha256(data),
                "recompiled_file_present": have,
                "recompiled_sha256": sha256(rebuilt) if have else None,
                "byte_identical": have and data == rebuilt,
            })
    report = base_report(head, "custom PyInstaller-6.22.0 win_amd64 bootloader")
    matched = sum(x["byte_identical"] for x in results)
    report.update({
        "custom_wheel_sha256_verified": PY_WHEEL_SHA,
        "source_git_commit_verified": PY_SHA,
        "custom_builder_git_commit_verified": PYBUILD_SHA,
        "wheel_metadata_path": meta,
        "bootloader_images": results,
        "recompiled_image_count": sum(x["recompiled_file_present"] for x in results),
        "byte_identical_image_count": matched,
        "all_recompiled_bootloader_bytes_identical": matched == len(results),
        "comparison_scope": "Windows-64bit-intel bootloader PE members only",
        "known_limitations": [
            "Rebuild uses current MSYS2 packages, not an attested historical compiler environment",
            "Different bytes can reflect compiler flags, build paths, timestamps, or toolchain drift",
            "Matching bytes do not establish full Python-wheel transitive corresponding source",
            "EXE was read as inert bytes; no downloaded wheel or PE was executed",
            "Neither release, publisher signature, nor legal approval is granted",
        ],
    })
    return report

def analyze_ejs(wheel_path, sdist_path, root, head):
    need(git_head(root) == EJS_SHA, "EJS source checkout commit drift")
    need(file_sha(wheel_path) == EJS_WHEEL_SHA and
         file_sha(sdist_path) == EJS_SDIST_SHA,
         "EJS published wheel/sdist SHA differs")
    sdist_matches = []
    with checked_sdist(sdist_path) as tar:
        for rel in EJS_SOURCE_PATHS:
            member = tar.getmember("yt_dlp_ejs-0.8.0/" + rel)
            need(member.isfile() and (root / rel).is_file(),
                 "pinned EJS sdist source missing " + rel)
            body = tar.extractfile(member).read()
            need(len(body) <= 4 * 1024 * 1024, "oversized sdist source file")
            same = body == (root / rel).read_bytes()
            sdist_matches.append({"path": rel, "byte_identical": same,
                                  "sdist_sha256": sha256(body)})
    need(all(x["byte_identical"] for x in sdist_matches),
         "published EJS sdist does not contain exact pinned Git source")
    results = []
    with checked_zip(wheel_path) as wheel:
        meta = metadata_match(wheel, "yt-dlp-ejs", "0.8.0")
        for member, rel in EJS_JS.items():
            need(member in wheel.namelist(), "published EJS wheel JS missing " + member)
            built_path = root / rel
            need(built_path.is_file(), "Deno build has no output " + rel)
            installed = wheel.read(member)
            rebuilt = built_path.read_bytes()
            results.append({
                "wheel_member": member, "wheel_sha256": sha256(installed),
                "rebuilt_sha256": sha256(rebuilt),
                "byte_identical": installed == rebuilt,
            })
    report = base_report(head, "yt-dlp-ejs 0.8.0")
    report.update({
        "pypi_wheel_sha256_verified": EJS_WHEEL_SHA,
        "pypi_sdist_sha256_verified": EJS_SDIST_SHA,
        "source_git_commit_verified": EJS_SHA,
        "sdist_source_files_identical_to_git": len(sdist_matches),
        "sdist_sources": sdist_matches,
        "wheel_metadata_path": meta,
        "js_bundle_comparisons": results,
        "both_wheel_js_bundles_byte_identical_to_deno_build":
            all(x["byte_identical"] for x in results),
        "rebuild_tool": "Deno 2.6.3 from upstream pinned release workflow",
        "pypi_publish_attestation_independently_verified": False,
        "known_limitations": [
            "Published PyPI provenance was observed separately, not cryptographically verified here",
            "Deno rebuild tests EJS first-party JS bundle equality, not full dependencies' source",
            "Wheel SHA verification does not prove exact inclusion in the outer yt-dlp.exe",
            "No separate source tarballs for astring and meriyah are included in 01v",
            "Neither legal approval nor release is authorized",
        ],
    })
    return report

def save_report(report, target):
    with target.open("x", encoding="utf8") as out:
        json.dump(report, out, sort_keys=True, indent=2)
        out.write("\n")

def cli(argv):
    need(len(argv) >= 3, "usage: script download-pyi|download-ejs|pyinstaller|ejs allowlist ...")
    kind, allow_path = argv[1:3]
    head, temp = common_guard(allow_path)
    if kind == "download-pyi":
        need(len(argv) == 3, "download-pyi arguments")
        n = download_pyi(temp)
        print("VIDEO_TOOLS_01V_PYI_DOWNLOAD_PASS sha256=1 bytes=" + str(n))
    elif kind == "download-ejs":
        need(len(argv) == 3, "download-ejs arguments")
        n = download_ejs(temp)
        print("VIDEO_TOOLS_01V_EJS_DOWNLOAD_PASS packages=" + str(n))
    elif kind == "pyinstaller":
        need(len(argv) == 6, "pyinstaller arguments")
        _, _, _, source_path, builder_path, output_path = argv
        output = require_temp(output_path, temp, "video-tools-01v-pyi-NOT-APPROVAL.json", False)
        wheel = require_temp(temp / PY_WHEEL_NAME, temp, PY_WHEEL_NAME)
        result = analyze_pyinstaller(wheel, Path(source_path), Path(builder_path), head)
        save_report(result, output)
        print("VIDEO_TOOLS_01V_PYI_INSPECTION_PASS wheel_sha=1 images=" +
              str(len(result["bootloader_images"])) + " rebuilt=" +
              str(result["recompiled_image_count"]) + " byte_equal=" +
              str(result["byte_identical_image_count"]) +
              " complete_source=0 legal_approval=0 release=0")
    elif kind == "ejs":
        need(len(argv) == 5, "ejs arguments")
        _, _, _, source_path, output_path = argv
        output = require_temp(output_path, temp, "video-tools-01v-ejs-NOT-APPROVAL.json", False)
        wheel = require_temp(temp / EJS_WHEEL_NAME, temp, EJS_WHEEL_NAME)
        sdist = require_temp(temp / EJS_SDIST_NAME, temp, EJS_SDIST_NAME)
        result = analyze_ejs(wheel, sdist, Path(source_path), head)
        save_report(result, output)
        print("VIDEO_TOOLS_01V_EJS_INSPECTION_PASS wheel_sha=1 sdist_sha=1 "
              "sdist_git=1 js_equal=" +
              str(int(result["both_wheel_js_bundles_byte_identical_to_deno_build"])) +
              " complete_source=0 legal_approval=0 release=0")
    else:
        raise Hold("unknown mode")

if __name__ == "__main__":
    try:
        cli(sys.argv)
    except (Hold, STARTER.Hold, OSError, ValueError, KeyError, IndexError,
            subprocess.SubprocessError, urllib.error.URLError, tarfile.TarError,
            zipfile.BadZipFile) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
