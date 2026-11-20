#!/usr/bin/env python3
"""Fail-closed tests with synthetic PyInstaller-compatible CArchive and PYZ."""
import importlib.util
import marshal
import struct
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "video_tools_01r_inventory", HERE / "video-tools-01r-pe-archive-inventory.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def member(name, kind, payload, offset):
    name = name.encode("utf8") + b"\0"
    entry_len = MODULE.ENTRY.size + len(name)
    padding = (-entry_len) % 16
    entry_len += padding
    return (MODULE.ENTRY.pack(entry_len, offset, len(payload), len(payload), 0,
                              kind.encode("ascii")) + name + b"\0" * padding)


def make_image(duplicate=False, omit_pyz=False):
    names = {"yt_dlp": (0, 12, 1), "yt_dlp.extractor.youtube": (0, 12, 1)}
    pyz = b"PYZ\0" + b"\x42\x0d\x0d\x0a" + struct.pack(">i", 13)
    pyz += b"x" + marshal.dumps(names)
    pieces = []
    toc = b""
    if not omit_pyz:
        toc += member("PYZ-00.pyz", "z", pyz, len(b"".join(pieces)))
        pieces.append(pyz)
    dll = b"MZ" + b"\x00" * 18
    toc += member("python312.dll", "b", dll, len(b"".join(pieces)))
    pieces.append(dll)
    if duplicate:
        toc += member("python312.dll", "b", dll, len(b"".join(pieces)))
        pieces.append(dll)
    data = b"".join(pieces)
    cookie = MODULE.COOKIE.pack(MODULE.MAGIC, len(data) + len(toc) + MODULE.COOKIE.size,
                                len(data), len(toc), 312, b"python312.dll".ljust(64, b"\0"))
    return b"MZ" + b"\0" * 1022 + data + toc + cookie


class InventoryTest(unittest.TestCase):
    def test_real_format_synthetic_nested_archive(self):
        result = MODULE.parse_carchive(make_image())
        self.assertEqual(result["carchive_count"], 2)
        self.assertEqual(result["python_module_count"], 2)
        self.assertEqual(result["python_version_code"], 312)
        self.assertEqual([v["name"] for v in result["embedded_python_items"]],
                         ["yt_dlp", "yt_dlp.extractor.youtube"])

    def test_mutations_fail_closed(self):
        for label, payload in [
            ("no MZ", b"XX" + make_image()[2:]),
            ("bad cookie", make_image()[:-MODULE.COOKIE.size]),
            ("missing PYZ", make_image(omit_pyz=True)),
            ("duplicate", make_image(duplicate=True)),
        ]:
            with self.subTest(label=label):
                with self.assertRaises(MODULE.Hold):
                    MODULE.parse_carchive(payload)

    def test_reject_bad_path_or_corrupt_pyarchive(self):
        for name in ("../outside", "C:\\evil", "/etc", "", "..\\evil"):
            with self.subTest(name=name):
                with self.assertRaises(MODULE.Hold):
                    MODULE.safe_name(name)
        with self.assertRaises(MODULE.Hold):
            MODULE.parse_pyz(b"PYZ\0" + b"x" * 10)
        with self.assertRaises(MODULE.Hold):
            MODULE.parse_pyz(b"PYZ\0" + b"\x00" * 4 + struct.pack(">i", 12) + b"?")

    def test_release_admission_and_unpinned_exe_are_rejected(self):
        fake = make_image()
        allow = {
            "schema": 1, "release_admitted": False,
            "installer_signing_authorized": False,
            "stage_binary_distribution_authorized": False,
            "independent_owner_approval_required": True,
            "components": [{
                "name": "akula-yt-dlp.exe", "license": "GPL-3.0-or-later",
                "source_license": "Unlicense", "license_status": "REVIEW_BUNDLED_DEPENDENCIES",
                "reference": MODULE.YT_TAG, "tag_commit_sha": MODULE.YT_COMMIT,
                "sha256": MODULE.YT_SHA, "size_bytes": MODULE.YT_SIZE,
            }]
        }
        with self.assertRaisesRegex(MODULE.Hold, "SHA-256"):
            MODULE.make_report(allow, fake)
        allow["release_admitted"] = True
        with self.assertRaisesRegex(MODULE.Hold, "release gate"):
            MODULE.make_report(allow, fake)
        allow["release_admitted"] = False
        allow["components"][0]["license"] = "Unlicense"
        with self.assertRaisesRegex(MODULE.Hold, "license distinction"):
            MODULE.make_report(allow, fake)

    def test_pr_only_workflow_cannot_publish_exe_or_license_approval(self):
        workflow = (HERE.parent / "workflows" /
                    "akula-video-tools-01r-embedded-review.yml").read_text("utf8")
        docs = (HERE.parent.parent / "docs/release" /
                "video-tools-01r-binary-constituents.md").read_text("utf8")
        checks = [
            "runs-on: ubuntu-24.04",
            "      - '.github/scripts/video-tools-01r-pe-archive-inventory.py'",
            "      - '.github/scripts/video-tools-01r-pe-archive-inventory.test.py'",
            "      - 'docs/release/video-tools-01r-binary-constituents.md'",
            "python3 .github/scripts/video-tools-01r-pe-archive-inventory.test.py",
            "video-tools-01r-embedded-names-HOLD.json",
            "VIDEO_TOOLS_01R_REAL_PYINSTALLER_INVENTORY_PASS",
            "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d",
            "retention-days: 1",
            "permissions:\n  contents: read",
            "persist-credentials: false",
        ]
        for token in checks:
            self.assertIn(token, workflow, "VIDEO_TOOLS_01R_HOLD missing: " + token)
        for forbidden in ("workflow_dispatch:", "\n  push:", "contents: write",
                          "gh release create", "upload-release-asset"):
            self.assertNotIn(forbidden, workflow)
        for token in ("VIDEO-18 RELEASE HOLD", "not a complete SBOM",
                      "corresponding source", "GPLv3+", "No Windows EXE"):
            self.assertIn(token, docs)
        self.assertNotIn("release_admitted=true", docs)


if __name__ == "__main__":
    unittest.main(verbosity=2)
