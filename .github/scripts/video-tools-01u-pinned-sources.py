#!/usr/bin/env python3
"""VIDEO-TOOLS-01u: extend 01t with pinned PyInstaller and EJS source archives.

The output is a NON-COMPLETE evidence add-on. It does not contain executable
wheels, attest any rebuild, or determine legal license compliance.
"""
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "video_tools_01t", HERE / "video-tools-01t-source-starter.py"
)
STARTER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STARTER)

PY_SHA = "70fc17210920bce17f4ab09bbf8104b0dbd45338"
EJS_SHA = "4fb477f4af56880cfd324c48bd4294a2d2294e50"
EJS_WHEEL_SHA = "79300e5fca7f937a1eeede11f0456862c1b41107ce1d726871e0207424f4bdb4"
EJS_SDIST_SHA = "d5fa1639f63b5c4af8d932495f60689d5370f1a095782c944f7f62a303eb104e"
PY_BLOBS = {
    "COPYING.txt": "51bf4bd138e68f0ccafce4fbf7723842dceaa1d1",
    "README.rst": "a418b9c502c0519144197d938b5da440ced57696",
    "bootloader/wscript": "e86be20a051bdc463977790bc73395db6c01d4eb",
    "pyproject.toml": "f1e45d51131f93f0d2440e9b9719ba3f7f9657a2",
}
EJS_BLOBS = {
    "LICENSE": "efb98088164f5786b17e83ed384971fc3c74f93c",
    "README.md": "50d7de2b8c43e6460eed9e1c3e3dc6efeab340dc",
    "pyproject.toml": "a2968e19c1ee20e1c97dac587de86a014d70ca5d",
    "package.json": "6c7ae87eb2e8a18cb6e7ebcd68655be8b4c0aba2",
    "package-lock.json": "dd460c7d4ce3557f0ed2fba0f97486e6fcf96a06",
    "hatch_build.py": "54fffb634dbcc5f7d834e3d7491a06c5fb49a608",
    "rollup.config.js": "ea6cffc15ac1aba2099b5a6d01a0005ab9706643",
    "src/yt/solver/main.ts": "6de821917e06366b4b7381f35e5a89f8f54fd6ce",
}
PY_FILES = [
    "COPYING.txt", "README.rst", "MANIFEST.in", "pyproject.toml",
    "hatch_build.py", "bootloader",
    "PyInstaller/__init__.py", "PyInstaller/__main__.py",
    "PyInstaller/_recursion_too_deep_message.py", "PyInstaller/_shared_with_waf.py",
    "PyInstaller/archive", "PyInstaller/building", "PyInstaller/compat.py",
    "PyInstaller/config.py", "PyInstaller/configure.py", "PyInstaller/depend",
    "PyInstaller/exceptions.py", "PyInstaller/fake-modules", "PyInstaller/hooks",
    "PyInstaller/isolated", "PyInstaller/lib", "PyInstaller/loader",
    "PyInstaller/log.py", "PyInstaller/utils",
]
EXPECTED_PY_FILES = [
    "COPYING.txt", "README.rst", "bootloader/wscript",
    "bootloader/src/pyi_main.c", "PyInstaller/__main__.py",
    "PyInstaller/loader/pyimod01_archive.py",
]
EXPECTED_EJS_FILES = [
    "LICENSE", "README.md", "package.json", "package-lock.json",
    "pyproject.toml", "hatch_build.py", "rollup.config.js",
    "src/yt/solver/main.ts",
]
OUTPUT = {
    "pyinstaller": "pyinstaller-6.22.0-SOURCE-ONLY.tar.gz",
    "ejs": "yt-dlp-ejs-0.8.0-SOURCE-ONLY.tar.gz",
}


def need(value, reason):
    if not value:
        raise STARTER.Hold("VIDEO_TOOLS_01U_HOLD: " + reason)


def verify_blobs(root, expected):
    rows = []
    for rel, sha in sorted(expected.items()):
        path = root / rel
        need(path.is_file() and not path.is_symlink(), "source file missing or linked " + rel)
        blob = path.read_bytes()
        need(STARTER.git_blob(blob) == sha, "pinned source blob SHA drift " + rel)
        rows.append({"path": rel, "git_blob_sha1": sha, "bytes": len(blob)})
    return rows


def verify_source_metadata(py, ejs):
    copying = (py / "COPYING.txt").read_text("utf8")
    need("Bootloader Exception" in copying and
         "GNU General Public License" in copying and
         "Run-time Hooks" in copying, "PyInstaller mixed-license notice missing")
    pyproject = (py / "pyproject.toml").read_text("utf8")
    need("GPLv2-or-later with a special exception" in pyproject,
         "PyInstaller bootloader exception changed")
    boot = (py / "bootloader/wscript").read_text("utf8")
    need("GPL-2.0-or-later WITH Bootloader-exception" in boot,
         "PyInstaller bootloader source license changed")
    metadata = (ejs / "pyproject.toml").read_text("utf8")
    need('license = "Unlicense AND MIT AND ISC"' in metadata,
         "EJS bundled wheel licenses no longer match pinned tag")
    pkg = json.loads((ejs / "package.json").read_text("utf8"))
    need(pkg.get("dependencies") == {"astring": "1.9.0", "meriyah": "6.1.4"},
         "EJS bundled JavaScript dependencies changed")
    readme = (ejs / "README.md").read_text("utf8")
    need("meriyah" in readme and "astring" in readme and
         "Unlicense" in readme and "MIT" in readme and "ISC" in readme,
         "EJS upstream license declarations missing")
    lock = json.loads((ejs / "package-lock.json").read_text("utf8"))
    packages = lock.get("packages", {})
    need(packages.get("node_modules/meriyah", {}).get("version") == "6.1.4" and
         packages.get("node_modules/astring", {}).get("version") == "1.9.0",
         "EJS package lock differs from license-bearing pinned JS modules")
    for path in ("node_modules/meriyah", "node_modules/astring"):
        need(bool(re.fullmatch(r"sha512-[A-Za-z0-9+/=]+",
              packages[path].get("integrity", ""))),
              "EJS dependency has no locked integrity " + path)
    return {
        "pyinstaller": {
            "upstream_declared_license": "GPL-2.0-or-later WITH Bootloader-exception",
            "runtime_hooks": "Apache-2.0; separate source review needed",
            "bootloader_compiled_wheel_equivalence_verified": False,
        },
        "ejs": {
            "source_tree_license": "Unlicense",
            "built_wheel_declared_license": "Unlicense AND MIT AND ISC",
            "embedded_js_dependencies": {
                name: {"version": packages["node_modules/" + name]["version"],
                       "npm_integrity": packages["node_modules/" + name]["integrity"]}
                for name in ("astring", "meriyah")
            },
            "pypi_wheel_sha256_from_upstream_release_metadata": EJS_WHEEL_SHA,
            "pypi_sdist_sha256_from_upstream_release_metadata": EJS_SDIST_SHA,
            "pypi_artifact_bytes_checked": False,
            "wheel_vs_embedded_js_equivalence_verified": False,
        },
    }


def verify_base_starter(manifest, exact_sha):
    need(manifest.get("schema") == 1 and
         manifest.get("exact_akula_head") == exact_sha and
         manifest.get("release_admitted") is False and
         manifest.get("legal_approval") is False and
         manifest.get("complete_corresponding_source_available") is False and
         manifest.get("corresponding_source_rebuilt_and_verified") is False and
         manifest.get("windows_executables_included") is False,
         "01t source-starter accepted release or did not match exact head")
    snaps = manifest.get("snapshots", {})
    need(set(snaps) == {"yt", "quickjs", "pybuild", "akula"} and
         all(re.fullmatch(r"[a-f0-9]{64}", value.get("sha256", "")) and
             value.get("tracked_file_count", 0) > 5 for value in snaps.values()),
         "01t four pinned Git snapshots missing")


def build(py, ejs, base_manifest, head):
    need(STARTER.git_head(py) == PY_SHA and STARTER.git_head(ejs) == EJS_SHA,
         "source checkout differs from immutable commits")
    verify_base_starter(base_manifest, head)
    evidence = {
        "pyinstaller": verify_blobs(py, PY_BLOBS),
        "ejs": verify_blobs(ejs, EJS_BLOBS),
    }
    licensing = verify_source_metadata(py, ejs)
    archives = {}
    snapshots = {}
    for key, root, sha, selections, must in [
        ("pyinstaller", py, PY_SHA, PY_FILES, EXPECTED_PY_FILES),
        ("ejs", ejs, EJS_SHA, [], EXPECTED_EJS_FILES),
    ]:
        archive, count = STARTER.archive_tracked(
            root, sha, key + "-sources", selections, must
        )
        archives[key] = archive
        snapshots[key] = {
            "filename": OUTPUT[key], "git_commit": sha,
            "source_files_included": count, "sha256": hashlib.sha256(archive).hexdigest(),
            "bytes": len(archive), "contents": "Git-tracked text/build sources, no prebuilt Windows EXEs",
        }
    need(snapshots["pyinstaller"]["source_files_included"] > 250 and
         snapshots["ejs"]["source_files_included"] >= 15,
         "source snapshot unexpectedly sparse")
    return {
        "schema": 1, "stage": "VIDEO-TOOLS-01u add-on to 01t / NOT COMPLETE",
        "exact_akula_head": head, "release_admitted": False,
        "legal_approval": False, "complete_corresponding_source_available": False,
        "corresponding_source_rebuilt_and_verified": False,
        "publisher_signing_authorized": False, "windows_executables_included": False,
        "complete_license_notice_package": False, "binary_sbom_complete": False,
        "verified_01t_source_tree_hashes": {
            key: value["sha256"] for key, value in base_manifest["snapshots"].items()
        },
        "supplemental_source_snapshots": snapshots,
        "pinned_git_blobs": evidence, "licenses_and_dependency_pins": licensing,
        "still_missing": [
            "Correspondence between custom Pyinstaller-Builds MinGW wheel and upstream PyInstaller bootloader executable was not rebuilt or byte-matched",
            "Pinned PyInstaller build dependencies' actual source distributions were not acquired",
            "yt-dlp-ejs 0.8.0 source and lock were acquired, but its published wheel/source-to-bundled JavaScript equality was not verified",
            "EJS meriyah and astring upstream corresponding source tarballs and their exact built JS correspondence are not included",
            "Exact Python 3.14, curl-cffi, OpenSSL, native DLL/PYD and other hashed Python wheel sources remain missing",
            "PyInstaller/bootloader compiled executable fixtures are excluded from source archive by design",
            "Rust Cargo full transitive corresponding source, QuickJS native dependency review and notice delivery remain unverified",
            "Independent legal and owner review has not approved any distribution",
        ],
    }, archives


README = """VIDEO-TOOLS-01u: ADD-ON to VIDEO-TOOLS-01t source starter.
Contains pinned Git sources for PyInstaller 6.22.0 and yt-dlp/ejs 0.8.0.
No wheels, no exe, no DLL, no runtime code execution, no release.
For reproducibility use the 01t four-archive starter from SAME exact PR HEAD.
This is NOT complete corresponding source for GPLv3+/AGPL, not a legal approval.
The PyInstaller bootloader wheel is NOT rebuilt or source-byte matched.
The EJS wheel with meriyah ISC / astring MIT is NOT independently rebuilt.
See manifest-01u-NOT-COMPLETE.json and remaining source obligations.
VIDEO-18 RELEASE HOLD.
"""


def cli(argv):
    need(len(argv) == 7,
         "usage: 01u-script allowlist pyinstaller-dir ejs-dir 01t-manifest head output-dir")
    _, allow_path, pyroot, ejsroot, mpath, head, target = argv
    need(os.environ.get("GITHUB_ACTIONS") == "true" and
         os.environ.get("GITHUB_EVENT_NAME") == "pull_request" and
         re.fullmatch(r"[0-9a-f]{40}", head) is not None and
         head == os.environ.get("EXPECTED_HEAD_SHA"),
         "only exact-HEAD pull_request evidence is supported")
    akula = Path(os.environ.get("GITHUB_WORKSPACE", "")).resolve(strict=True)
    temp = Path(os.environ.get("RUNNER_TEMP", "")).resolve(strict=True)
    out = Path(target).resolve(strict=False)
    need(STARTER.git_head(akula) == head and
         out.parent == temp and out.name == "video-tools-01u-source-addon-HOLD" and
         not out.exists(), "working checkout or output directory is not exact/clean")
    py = Path(pyroot).resolve(strict=True)
    ejs = Path(ejsroot).resolve(strict=True)
    need(len({py, ejs, akula}) == 3 and
         py != temp and ejs != temp,
         "upstream checkout isolation absent")
    allow = json.loads(Path(allow_path).read_text("utf8"))
    STARTER.check_release_hold(allow)
    raw = Path(mpath).read_text("utf8")
    base = json.loads(raw)
    result, archives = build(py, ejs, base, head)
    out.mkdir(mode=0o700)
    for key, archive in archives.items():
        (out / OUTPUT[key]).write_bytes(archive)
    (out / "manifest-01u-NOT-COMPLETE.json").write_text(
        json.dumps(result, sort_keys=True, indent=2) + "\n", "utf8"
    )
    (out / "README-NOT-COMPLETE.txt").write_text(README, "utf8")
    need(sorted(x.name for x in out.iterdir()) ==
         sorted([*OUTPUT.values(), "manifest-01u-NOT-COMPLETE.json",
                 "README-NOT-COMPLETE.txt"]),
         "source add-on unexpectedly contains executable or untracked files")
    print("VIDEO_TOOLS_01U_GIT_SOURCES_PASS sources=2 "
          "pyinstaller_git=1 ejs_git=1 wheel_bytes=0 "
          "complete_source=0 rebuilt_exe=0 legal_approval=0 release=0")


if __name__ == "__main__":
    try:
        cli(sys.argv)
    except (STARTER.Hold, OSError, KeyError, ValueError, subprocess.SubprocessError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(3)
