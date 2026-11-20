#!/usr/bin/env python3
"""VIDEO-TOOLS-01v fail-closed tests. No network, downloads or binaries."""
import importlib.util
import io
import json
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("v01", HERE/"video-tools-01v-wheel-provenance.py")
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def mkzip(path, files):
    with zipfile.ZipFile(path, "w") as z:
        for name, body in files.items():
            z.writestr(name, body)

class ProvenanceTests(unittest.TestCase):
    def test_exact_pins_and_no_release(self):
        self.assertEqual(M.PY_SHA, "70fc17210920bce17f4ab09bbf8104b0dbd45338")
        self.assertEqual(M.EJS_SHA, "4fb477f4af56880cfd324c48bd4294a2d2294e50")
        self.assertEqual(M.PY_WHEEL_SHA, "294099ecb5fdd2a13ae4c29006d4e335b697a63b6a16cf052b90ec2b40bef05a")
        self.assertEqual(M.EJS_WHEEL_SHA, "79300e5fca7f937a1eeede11f0456862c1b41107ce1d726871e0207424f4bdb4")
        report = M.base_report("a"*40, "test")
        for key in ("legal_approval", "release_admitted",
                    "complete_corresponding_source_available",
                    "corresponding_source_rebuilt_and_verified",
                    "publisher_signing_authorized", "binary_sbom_complete"):
            self.assertIs(report[key], False)

    def test_paths_and_zip_bounds(self):
        for path in ("../escape", "/abs", "a\\b", "a/../b", "", "a//b"):
            self.assertFalse(M.safe_name(path), path)
        for path in ("a/b/c", "p.dist-info/METADATA", "a/"):
            self.assertTrue(M.safe_name(path), path)
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp)/"wheel.whl"
            mkzip(p, {"pkg/x.txt": "hello", "pkg.dist-info/METADATA":
                          "Name: example\nVersion: 1.0\n"})
            with M.checked_zip(p) as z:
                self.assertTrue(M.metadata_match(z, "example", "1.0"))
                with self.assertRaises(M.Hold):
                    M.metadata_match(z, "example", "2.0")
            mkzip(p, {"../evil": "no"})
            with self.assertRaisesRegex(M.Hold, "unsafe"):
                M.checked_zip(p)
            mkzip(p, {"large": "a"*1000})
            with self.assertRaisesRegex(M.Hold, "size"):
                M.checked_zip(p, 100)

    def test_ejs_wheel_requires_two_bundles_and_exact_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root/"dist").mkdir()
            (root/"dist/yt.solver.core.min.js").write_bytes(b"exact core")
            (root/"dist/yt.solver.lib.min.js").write_bytes(b"exact lib")
            wheel = root/M.EJS_WHEEL_NAME
            mkzip(wheel, {
                "yt_dlp_ejs/yt/solver/core.min.js": b"exact core",
                "yt_dlp_ejs/yt/solver/lib.min.js": b"exact lib",
                "yt_dlp_ejs-0.8.0.dist-info/METADATA": "Name: yt-dlp-ejs\nVersion: 0.8.0\n",
            })
            self.assertEqual(M.sha256(wheel.read_bytes()), M.file_sha(wheel))
            with M.checked_zip(wheel) as z:
                M.metadata_match(z, "yt-dlp-ejs", "0.8.0")
                for entry, rel in M.EJS_JS.items():
                    self.assertEqual(z.read(entry), (root/rel).read_bytes())
            (root/"dist/yt.solver.lib.min.js").write_bytes(b"different")
            with M.checked_zip(wheel) as z:
                self.assertNotEqual(z.read("yt_dlp_ejs/yt/solver/lib.min.js"),
                                    (root/"dist/yt.solver.lib.min.js").read_bytes())

    def test_mismatched_and_absent_download_blocked(self):
        with tempfile.TemporaryDirectory() as temp:
            f = Path(temp)/"fake.whl"
            f.write_bytes(b"not a pinned wheel")
            with self.assertRaises(M.Hold):
                M.checked_zip(f)
            self.assertNotEqual(M.file_sha(f), M.PY_WHEEL_SHA)

if __name__ == "__main__":
    unittest.main()
