#!/usr/bin/env python3
"""VIDEO-TOOLS-01x: bounded, source-only PyPI acquisition for pinned build inputs.

Published sdists are read as inert archives, not installed/imported/executed.
A PyPI sdist is not proof of the actual native wheel's linked libraries or
complete corresponding source for a frozen Windows executable.
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
import urllib.error
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("video_tools_01t", HERE / "video-tools-01t-source-starter.py")
STARTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STARTER)

SOURCE_TARGETS = {
    "brotli": "1.2.1",
    "cffi": "2.1.1",
    "curl-cffi": "0.17.0",
    "mutagen": "1.48.1",
    "pycryptodomex": "3.24.1",
    "websockets": "17.3",
}
SOURCE_GROUP = "curl-cffi"
MAX_JSON = 4 * 1024 * 1024
MAX_FILE = 20 * 1024 * 1024
MAX_UNPACKED = 250 * 1024 * 1024
MAX_ITEMS = 15000
OUTPUT_DIR = "video-tools-01x-source-HOLD"
META_NAME = "manifest-01x-NOT-COMPLETE.json"
README_NAME = "README-NOT-COMPLETE.txt"

class Hold(Exception):
    pass

def need(test, message):
    if not test:
        raise Hold("VIDEO_TOOLS_01X_HOLD: " + message)

def canonical(name):
    need(isinstance(name, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", name) is not None,
         "invalid package name")
    return re.sub(r"[-_.]+", "-", name).lower()

def sha256(blob):
    return hashlib.sha256(blob).hexdigest()

def validate_filename(filename):
    need(isinstance(filename, str) and
         re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.+!~-]*\.(?:tar\.gz|zip)", filename) is not None and
         len(filename) <= 180, "unsafe source archive filename")
    return filename

def parse_all_locks(ytroot):
    groups = STARTER.validate_windows_inputs(ytroot)
    hashes = {}
    for group, rel in STARTER.REQ.items():
        current = None
        lock_hashes = {}
        text = (ytroot / rel).read_text("utf8")
        for raw in text.splitlines():
            line = raw.strip()
            m = re.match(r"^([A-Za-z][A-Za-z0-9_.-]*)==([0-9][A-Za-z0-9.+_-]*)\b", line)
            if m:
                current = canonical(m.group(1))
                lock_hashes[current] = set()
            elif line.startswith("pyinstaller @ "):
                current = "pyinstaller"
                lock_hashes[current] = set()
            elif line.startswith("--hash=sha256:"):
                need(current is not None, "orphan hash in " + group)
                m = re.fullmatch(r"--hash=sha256:([0-9a-f]{64})\s*\\?", line)
                need(m is not None, "malformed dependency pin " + group)
                lock_hashes[current].add(m.group(1))
        need(set(lock_hashes) == set(groups[group]), "requirements parser drift in " + group)
        for name, pin in groups[group].items():
            need(len(lock_hashes[name]) == pin["hash_count"], "hash count mismatch " + name)
        hashes[group] = lock_hashes
    need(sum(len(rows) for rows in groups.values()) == 23,
         "Windows build input count changed; explicitly review selection")
    need(len(groups[SOURCE_GROUP]) == 14, "curl-cffi source lock group changed")
    for name, version in SOURCE_TARGETS.items():
        need(groups[SOURCE_GROUP].get(name, {}).get("version") == version,
             "native/third-party source version pin changed " + name)
    return groups, hashes

def compatible_cpython314_win64(filename):
    """Only possible *metadata* wheel candidates for normal CPython 3.14 x64.

    Does not attest the chosen installed wheel, OS loader or native contents.
    Reject CPython free-threading (cp314t), PyPy and newer cp315 ABI wheels.
    """
    if not isinstance(filename, str) or not filename.endswith(".whl"):
        return False
    parts = filename[:-4].rsplit("-", 3)
    if len(parts) != 4:
        return False
    _, py, abi, platform = parts
    if platform not in ("win_amd64", "any"):
        return False
    py_tags, abi_tags = set(py.split(".")), set(abi.split("."))
    if platform == "any" and "none" in abi_tags and (
            "py3" in py_tags or "py314" in py_tags):
        return True
    if platform != "win_amd64":
        return False
    if "cp314" in py_tags and ("cp314" in abi_tags or "abi3" in abi_tags):
        return True
    for tag in py_tags:
        match = re.fullmatch(r"cp3([7-9]|1[0-3])", tag)
        if match and "abi3" in abi_tags:
            return True
    return False

def read_pypi_json(name, version):
    url = "https://pypi.org/pypi/" + urllib.parse.quote(name) + "/" + urllib.parse.quote(version) + "/json"
    request = urllib.request.Request(url, headers={
        "User-Agent": "Akula-01x-source-only-audit (nonrelease)",
        "Accept": "application/json",
    })
    with urllib.request.urlopen(request, timeout=35) as response:
        need(urllib.parse.urlparse(response.geturl()).hostname == "pypi.org",
             "PyPI metadata origin changed")
        raw = response.read(MAX_JSON + 1)
    need(50 < len(raw) <= MAX_JSON, "PyPI metadata size unbounded")
    data = json.loads(raw)
    info = data.get("info", {})
    need(canonical(info.get("name", "")) == name and info.get("version") == version,
         "PyPI package metadata name/version mismatched")
    need(isinstance(data.get("urls"), list), "PyPI release files absent")
    return data

def select_source(meta, name, version):
    sources = [z for z in meta["urls"] if z.get("packagetype") == "sdist"]
    if not sources:
        return None
    need(len(sources) == 1, "multiple sdists require independent review " + name)
    src = sources[0]
    fn = validate_filename(src.get("filename"))
    root = fn[: -len(".tar.gz")] if fn.endswith(".tar.gz") else fn[:-4]
    need(canonical(root.rsplit("-", 1)[0]) == name and root.endswith("-" + version),
         "unexpected PyPI source filename/version " + name)
    digest = src.get("digests", {}).get("sha256", "")
    need(re.fullmatch(r"[0-9a-f]{64}", digest) is not None,
         "PyPI source lacks SHA-256 " + name)
    url = urllib.parse.urlparse(src.get("url", ""))
    need(url.scheme == "https" and url.hostname == "files.pythonhosted.org",
         "source URL host not PyPI files")
    return src

def read_and_hash_source(src, output):
    filename = validate_filename(src["filename"])
    need(output.is_dir() and output.stat().st_mode & 0o077 == 0,
         "source output directory not restricted")
    target = output / filename
    need(not target.exists() and target.parent == output, "source destination not fresh")
    req = urllib.request.Request(src["url"], headers={
        "User-Agent": "Akula-01x-source-only-audit (nonrelease)",
    })
    count = 0
    checksum = hashlib.sha256()
    try:
        with urllib.request.urlopen(req, timeout=90) as response, target.open("xb") as f:
            need(urllib.parse.urlparse(response.geturl()).hostname == "files.pythonhosted.org",
                 "source URL redirect origin changed")
            while True:
                block = response.read(65536)
                if not block:
                    break
                count += len(block)
                need(count <= MAX_FILE, "source distribution exceeds 20MiB")
                checksum.update(block)
                f.write(block)
        need(count >= 1024 and checksum.hexdigest() == src["digests"]["sha256"],
             "downloaded source bytes do not match PyPI SHA-256")
        return target, count
    except BaseException:
        target.unlink(missing_ok=True)
        raise

def safe_member(name):
    return (isinstance(name, str) and len(name) < 500 and
            not name.startswith(("/", "\\")) and "\\" not in name and
            "\x00" not in name and ":" not in name and
            all(p not in ("", ".", "..") for p in name.rstrip("/").split("/")))

def inspect_source(path, name, version):
    filename = path.name
    meta = []
    seen = set()
    total = 0
    suffixes = {".c", ".h", ".cpp", ".cc", ".rs", ".pyx", ".pxd", ".cmake"}
    source_count = 0
    source_paths = []
    if filename.endswith(".tar.gz"):
        need(tarfile.is_tarfile(path), "not a source tar archive")
        with tarfile.open(path, "r:gz") as archive:
            members = archive.getmembers()
            need(len(members) <= MAX_ITEMS, "archive entries exceed bound")
            for m in members:
                need(safe_member(m.name), "unsafe source member path")
                need(m.isfile() or m.isdir(), "source archive contains links/devices: " + name)
                need(m.name not in seen, "duplicate source entry: " + name)
                seen.add(m.name)
                total += m.size
                need(total <= MAX_UNPACKED and m.size <= MAX_UNPACKED,
                     "source archive uncompressed size exceeds bound")
                if not m.isfile():
                    continue
                parts = m.name.split("/")
                need(len(parts) >= 2 and
                     canonical(parts[0].rsplit("-", 1)[0]) == name and
                     parts[0].endswith("-" + version), "source tar unexpected root")
                base = parts[-1].lower()
                if base in ("pkg-info", "pyproject.toml", "setup.py", "setup.cfg"):
                    meta.append("/".join(parts[1:]))
                if Path(base).suffix in suffixes:
                    source_count += 1
                    if len(source_paths) < 12:
                        source_paths.append("/".join(parts[1:]))
    else:
        need(zipfile.is_zipfile(path), "not a ZIP source archive")
        with zipfile.ZipFile(path) as archive:
            infos = archive.infolist()
            need(len(infos) <= MAX_ITEMS, "archive entries exceed bound")
            for m in infos:
                need(safe_member(m.filename), "unsafe ZIP source member")
                need(m.filename not in seen, "duplicate ZIP source entry")
                seen.add(m.filename)
                total += m.file_size
                need(total <= MAX_UNPACKED, "ZIP source exceeds size bound")
                if m.is_dir():
                    continue
                parts = m.filename.split("/")
                need(len(parts) >= 2 and
                     canonical(parts[0].rsplit("-", 1)[0]) == name and
                     parts[0].endswith("-" + version), "ZIP source unexpected root")
                base = parts[-1].lower()
                if base in ("pkg-info", "pyproject.toml", "setup.py", "setup.cfg"):
                    meta.append("/".join(parts[1:]))
                if Path(base).suffix in suffixes:
                    source_count += 1
    need(len(seen) > 3 and bool(meta), "source archive missing build/metadata files")
    return {
        "source_archive_file_count": len(seen),
        "unpacked_bytes_recorded": total,
        "build_metadata_paths": sorted(meta)[:30],
        "c_cxx_rust_cython_file_count": source_count,
        "example_source_paths": sorted(source_paths),
        "archive_executed": False,
        "complete_native_build_inputs_verified": False,
    }

def acquire(allow_path, ytroot, out_arg):
    head = os.environ.get("EXPECTED_HEAD_SHA", "")
    need(os.environ.get("GITHUB_ACTIONS") == "true" and
         os.environ.get("GITHUB_EVENT_NAME") == "pull_request" and
         re.fullmatch(r"[0-9a-f]{40}", head) is not None,
         "only exact PR HEAD action is allowed")
    akula = Path(os.environ.get("GITHUB_WORKSPACE", "")).resolve(strict=True)
    temp = Path(os.environ.get("RUNNER_TEMP", "")).resolve(strict=True)
    ytroot = Path(ytroot).resolve(strict=True)
    output = Path(out_arg).resolve(strict=False)
    need(STARTER.git_head(akula) == head and STARTER.git_head(ytroot) == STARTER.YT,
         "Akula or yt-dlp source identity changed")
    STARTER.check_release_hold(json.loads(Path(allow_path).read_text("utf8")))
    need(ytroot != akula and output == temp / OUTPUT_DIR and
         not output.exists(), "source output target not disposable/fresh")
    groups, hashes = parse_all_locks(ytroot)
    output.mkdir(mode=0o700)
    entries = []
    for name, version in sorted(SOURCE_TARGETS.items()):
        api = read_pypi_json(name, version)
        src = select_source(api, name, version)
        wheel_candidates = sorted([
            {"filename": row["filename"], "sha256": row["digests"]["sha256"]}
            for row in api["urls"]
            if row.get("packagetype") == "bdist_wheel" and
               re.fullmatch(r"[0-9a-f]{64}", row.get("digests", {}).get("sha256", "")) and
               row["digests"]["sha256"] in hashes[SOURCE_GROUP][name] and
               compatible_cpython314_win64(row.get("filename", ""))
        ], key=lambda z: z["filename"])
        row = {
            "distribution": name, "pinned_version": version,
            "requirements_group": SOURCE_GROUP,
            "accepted_wheel_or_sdist_hashes": len(hashes[SOURCE_GROUP][name]),
            "candidate_windows_wheels_from_pypi_metadata": wheel_candidates,
            "actual_wheel_downloaded_or_verified_in_exe": False,
            "actual_native_linked_libraries_identified": False,
            "corresponding_source_to_actual_wheel_proved": False,
            "source_archive_acquired": bool(src),
            "source_archive_sha256_in_upstream_lock": False,
        }
        if src is not None:
            file, size = read_and_hash_source(src, output)
            row.update({
                "source_filename": file.name,
                "source_sha256_verified": src["digests"]["sha256"],
                "source_bytes": size,
                "source_archive_sha256_in_upstream_lock":
                    src["digests"]["sha256"] in hashes[SOURCE_GROUP][name],
                "source_inspection": inspect_source(file, name, version),
            })
        else:
            row["blocker"] = "NO_PYPI_SDIST_FOR_EXACT_PIN; NEED_OTHER_SOURCE_PROVENANCE"
        entries.append(row)
    report = {
        "schema": 1,
        "stage": "VIDEO-TOOLS-01x source-only Python sdist procurement / NOT COMPLETE",
        "exact_akula_head": head,
        "yt_dlp_git_commit_verified": STARTER.YT,
        "pinned_group_package_counts": {k: len(v) for k, v in groups.items()},
        "pinned_build_install_records_total": sum(map(len, groups.values())),
        "selected_source_targets_count": len(SOURCE_TARGETS),
        "source_distributions_acquired_count": sum(x["source_archive_acquired"] for x in entries),
        "source_sdists_pinned_by_original_lock_count":
            sum(x["source_archive_sha256_in_upstream_lock"] for x in entries),
        "sources": entries,
        "remaining_gaps": [
            "Only 6 high-priority Python packages' published source archives are targeted; others remain unreviewed",
            "A source sdist from PyPI alone does not establish the source of the exact Windows wheel or its embedded DLLs",
            "curl-cffi may bundle external libcurl, OpenSSL/BoringSSL and curl-impersonate: native sources/patches unverified",
            "Python CPython 3.14 official source and interpreter embedding correspondence are not established",
            "PyInstaller custom MinGW bootloader 01w differs in .text/.rdata/.pdata from published wheel",
            "EJS 01v source-to-two-minified-JS matches exist separately; not linked to exact outer Windows EXE",
            "Nonselected Python build tools and all selected third-party notice/delivery reviews remain open",
            "Rust AGPL cargo/native transitive sources and QuickJS MIT notices remain separate",
        ],
        "release_admitted": False,
        "legal_approval": False,
        "complete_corresponding_source_available": False,
        "corresponding_source_rebuilt_and_verified": False,
        "publisher_signing_authorized": False,
        "binary_sbom_complete": False,
        "notices_packaging_approved": False,
        "windows_executables_included_in_source_artifact": False,
    }
    (output / META_NAME).write_text(json.dumps(report, sort_keys=True, indent=2) + "\n", "utf8")
    (output / README_NAME).write_text(
        "Akula VIDEO-TOOLS-01x: verified upstream PyPI sdists; NOT complete corresponding source.\n"
        "These files are copied without executing, extracting, compiling or installing them.\n"
        "Wheel-source equality, native DLL/CPython sources and license delivery remain unproved.\n"
        "No Windows EXE, DLL, wheel, installer or signing key is included.\n"
        "VIDEO-18 RELEASE HOLD: release=0 legal_approval=0.\n", "utf8")
    need(sorted(z.name for z in output.iterdir()) ==
         sorted([META_NAME, README_NAME] +
                [x["source_filename"] for x in entries if x["source_archive_acquired"]]),
         "source output unexpected files")
    print("VIDEO_TOOLS_01X_SOURCE_AUDIT_PASS packages=" + str(len(entries)) +
          " acquired=" + str(report["source_distributions_acquired_count"]) +
          " lock_sdists=" + str(report["source_sdists_pinned_by_original_lock_count"]) +
          " complete_source=0 legal_approval=0 release=0")

if __name__ == "__main__":
    try:
        need(len(sys.argv) == 4,
             "usage: source-audit.py allowlist.json yt-dlp-pinned-checkout output-directory")
        acquire(Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3])
    except (Hold, STARTER.Hold, OSError, ValueError, KeyError, TypeError,
            tarfile.TarError, zipfile.BadZipFile, urllib.error.URLError,
            subprocess.SubprocessError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
