#!/usr/bin/env python3
"""Synthetic, offline tests for 01x source-only pin and archive guards."""
import io
import importlib.util
import tarfile
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "video_tools_01x", HERE / "video-tools-01x-sdist-procurement.py")
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def make_tar(path, members):
    with tarfile.open(path, "w:gz") as out:
        for name, payload in members:
            body = payload.encode()
            info = tarfile.TarInfo(name)
            info.size = len(body)
            out.addfile(info, io.BytesIO(body))

class SdistSourceGuards(unittest.TestCase):
    def test_targets_fixed_and_no_release_flags(self):
        self.assertEqual(len(M.SOURCE_TARGETS), 6)
        self.assertEqual(M.SOURCE_TARGETS["curl-cffi"], "0.17.0")
        self.assertEqual(M.SOURCE_TARGETS["cffi"], "2.1.1")
        self.assertEqual(M.STARTER.CUSTOM_WHEEL_SHA,
                         "294099ecb5fdd2a13ae4c29006d4e335b697a63b6a16cf052b90ec2b40bef05a")
        self.assertIs(M.STARTER.check_release_hold is None, False)

    def test_pyname_and_sdist_selection(self):
        self.assertEqual(M.canonical("curl_cffi"), "curl-cffi")
        self.assertEqual(M.canonical("Curl.CFFI"), "curl-cffi")
        for candidate in ("/evil", "some/path", "../escape", "ab space", ""):
            with self.assertRaises(M.Hold):
                M.validate_filename(candidate)
        sample = {
            "urls": [{
                "packagetype": "sdist", "filename": "curl_cffi-0.17.0.tar.gz",
                "digests": {"sha256": "a" * 64},
                "url": "https://files.pythonhosted.org/packages/x/curl_cffi-0.17.0.tar.gz",
            }]
        }
        self.assertEqual(M.select_source(sample, "curl-cffi", "0.17.0")["filename"],
                         "curl_cffi-0.17.0.tar.gz")
        self.assertIsNone(M.select_source({"urls": []}, "cffi", "2.1.1"))
        bad = {"urls": [dict(sample["urls"][0], url="https://evil.example/a.tar.gz")]}
        with self.assertRaisesRegex(M.Hold, "host"):
            M.select_source(bad, "curl-cffi", "0.17.0")
        bad = {"urls": [dict(sample["urls"][0], filename="curl_cffi-0.18.0.tar.gz")]}
        with self.assertRaisesRegex(M.Hold, "filename"):
            M.select_source(bad, "curl-cffi", "0.17.0")
        bad = {"urls": sample["urls"] * 2}
        with self.assertRaisesRegex(M.Hold, "multiple"):
            M.select_source(bad, "curl-cffi", "0.17.0")

    def test_archive_introspection_safe_and_no_execution(self):
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp) / "cffi-2.1.1.tar.gz"
            make_tar(p, [
                ("cffi-2.1.1/PKG-INFO", "Name: cffi\nVersion: 2.1.1\n"),
                ("cffi-2.1.1/pyproject.toml", "[build-system]\n"),
                ("cffi-2.1.1/src/core.c", "int x;"),
                ("cffi-2.1.1/src/core.h", "#define X 1"),
            ])
            result = M.inspect_source(p, "cffi", "2.1.1")
            self.assertEqual(result["c_cxx_rust_cython_file_count"], 2)
            self.assertIs(result["archive_executed"], False)
            self.assertIs(result["complete_native_build_inputs_verified"], False)
            self.assertEqual(len(result["build_metadata_paths"]), 2)

    def test_traversal_missing_metadata_wrong_version_duplicate(self):
        with tempfile.TemporaryDirectory() as temp:
            p = Path(temp) / "cffi-2.1.1.tar.gz"
            for members, expected in [
                ([("../escape.txt", "evil"), ("cffi-2.1.1/PKG-INFO", "Name: cffi")], "unsafe"),
                ([("cffi-2.1.1/src/a.c", "a"), ("cffi-2.1.1/src/b.c", "b")], "metadata"),
                ([("cffi-2.1.2/PKG-INFO", "Name: cffi"), ("cffi-2.1.2/src/a.c", "a")], "root"),
                ([("cffi-2.1.1/PKG-INFO", "Name: cffi"),
                  ("cffi-2.1.1/PKG-INFO", "second")], "duplicate"),
            ]:
                make_tar(p, members)
                with self.assertRaisesRegex(M.Hold, expected):
                    M.inspect_source(p, "cffi", "2.1.1")

    def test_source_only_hash_is_not_wheel_proof(self):
        self.assertEqual(M.sha256(b"irrelevant"), "3c8acf66761354306466f2aa5c363549a20aa62e31d8c5fc67cddd997c6b846bb")
        self.assertFalse(M.safe_member("../evil"))
        self.assertFalse(M.safe_member("x\\../secret"))
        self.assertTrue(M.safe_member("pkg-1.2.3/src/main.c"))

if __name__ == "__main__":
    unittest.main()
