#!/usr/bin/env python3
"""VIDEO-TOOLS-01y synthetic and negative tests: no downloads or PE execution."""
import io
import importlib.util
import zipfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "video_tools_01y", HERE / "video-tools-01y-native-wheel-audit.py")
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def mk_wheel(name="curl_cffi", version="0.17.0", binary=b"native bytes"):
    f = io.BytesIO()
    with zipfile.ZipFile(f, "w") as z:
        z.writestr(f"{name}-{version}.dist-info/METADATA",
                   f"Metadata-Version: 2.3\nName: {name}\nVersion: {version}\n")
        z.writestr("curl_cffi/_wrapper.abi3.pyd", binary)
        z.writestr("curl_cffi/__init__.py", b"not executed")
    return f.getvalue()

class NativeWheelTests(unittest.TestCase):
    def test_static_targets_and_one_ambiguous_wheel_family(self):
        self.assertEqual(sum(M.CANDIDATES_EXPECTED.values()), 7)
        self.assertEqual(len(M.CANDIDATES_EXPECTED), 6)
        self.assertEqual(M.CANDIDATES_EXPECTED["websockets"], 2)
        self.assertEqual(M.STARTER.YT_EXE, M.R.YT_SHA)
        self.assertEqual(M.STARTER.YT, M.R.YT_COMMIT)

    def test_metadata_pinned_and_compatible_selection(self):
        url = "https://files.pythonhosted.org/packages/x/wheel.whl"
        native = {
            "filename": "curl_cffi-0.17.0-cp310-abi3-win_amd64.whl",
            "packagetype": "bdist_wheel",
            "digests": {"sha256": "a"*64},
            "url": url,
        }
        wrong = dict(native, filename="curl_cffi-0.17.0-cp310-abi3-manylinux_x86_64.whl")
        meta = {"info": {"name": "curl_cffi", "version": "0.17.0"},
                "urls": [native, wrong]}
        self.assertEqual([z["filename"] for z in M.wheel_candidates(
            meta, "curl-cffi", "0.17.0", {"a"*64})], [native["filename"]])
        with self.assertRaisesRegex(M.Hold, "candidate wheel set"):
            M.wheel_candidates(meta, "curl-cffi", "0.17.0", {"b"*64})
        with self.assertRaisesRegex(M.Hold, "release metadata"):
            M.wheel_candidates(meta, "curl-cffi", "0.18.0", {"a"*64})
        with self.assertRaisesRegex(M.Hold, "PyPI release host"):
            M.wheel_candidates(
                {"info":meta["info"], "urls":[dict(native,url="https://fake.example/x")]},
                "curl-cffi", "0.17.0", {"a"*64})

    def test_safe_wheel_zip_and_native_metadata(self):
        data = mk_wheel()
        parsed = M.wheel_native_members(data, "curl-cffi", "0.17.0")
        self.assertEqual(len(parsed), 1)
        self.assertEqual(parsed[0]["basename"], "_wrapper.abi3.pyd")
        self.assertEqual(parsed[0]["sha256"], M.sha256(b"native bytes"))
        with self.assertRaisesRegex(M.Hold, "metadata"):
            M.wheel_native_members(data, "curl-cffi", "0.18.0")
        for unsafe in ("../evil.pyd", "/tmp/escape.pyd", "curl_cffi/../../bad.dll"):
            z = io.BytesIO()
            with zipfile.ZipFile(z, "w") as wf:
                wf.writestr("curl_cffi-0.17.0.dist-info/METADATA",
                            "Name: curl-cffi\nVersion: 0.17.0\n")
                wf.writestr(unsafe, "bad")
            with self.assertRaisesRegex(M.Hold, "invalid wheel ZIP name"):
                M.wheel_native_members(z.getvalue(), "curl-cffi", "0.17.0")

    def test_corresponding_native_bytes_vs_names_only(self):
        expected = [{"wheel_member":"curl_cffi/_wrapper.abi3.pyd",
                     "sha256":M.sha256(b"exact"), "bytes":5,
                     "basename":"_wrapper.abi3.pyd"},
                    {"wheel_member":"curl_cffi/.libs/libcurl.dll",
                     "sha256":M.sha256(b"wheel"), "bytes":5,
                     "basename":"libcurl.dll"},
                    {"wheel_member":"curl_cffi/.libs/other.dll",
                     "sha256":M.sha256(b"missing"), "bytes":7,
                     "basename":"other.dll"}]
        native = [{"archive_member":"curl_cffi/_wrapper.abi3.pyd",
                   "sha256":M.sha256(b"exact"), "bytes":5,
                   "basename":"_wrapper.abi3.pyd"},
                  {"archive_member":"libcurl.dll",
                   "sha256":M.sha256(b"different"), "bytes":9,
                   "basename":"libcurl.dll"}]
        result = M.match_wheel_to_archive(expected, native)
        self.assertEqual(len(result["byte_matches"]), 1)
        self.assertEqual(len(result["same_name_different_bytes"]), 1)
        self.assertEqual(len(result["not_seen_in_native_archive"]), 1)
        self.assertFalse(result["full_wheel_installed_proven"])
        self.assertFalse(result["native_corresponding_source_verified"])
        self.assertFalse(result["native_wheel_member_set_fully_found_in_exe"])

    def test_no_matches_cannot_claim_complete_distribution(self):
        result = M.match_wheel_to_archive([], [])
        self.assertEqual(result["native_wheel_member_count"], 0)
        self.assertFalse(result["native_wheel_member_set_fully_found_in_exe"])
        self.assertFalse(result["full_wheel_installed_proven"])

if __name__ == "__main__":
    unittest.main()
