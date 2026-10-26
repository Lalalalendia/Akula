#!/usr/bin/env python3
"""Win7 dependency classification: dynamic CRT cannot be silently shipped."""
import runpy
from pathlib import Path
import unittest

audit = runpy.run_path(str(Path(__file__).with_name("akula-win7-dumpbin-audit.py")))
classify = audit["externally_deployed_crt_dlls"]


class CRTImportTests(unittest.TestCase):
    def test_reject_dynamic_ucrt_and_vcruntime(self):
        imports = {
            "kernel32.dll": ["CreateFileW"],
            "ole32.dll": ["CoTaskMemFree"],
            "api-ms-win-crt-runtime-l1-1-0.dll": ["_beginthreadex"],
            "api-ms-win-crt-heap-l1-1-0.dll": ["malloc"],
            "vcruntime140.dll": ["memcpy"],
            "msvcp140.dll": ["?Foo"],
            "ucrtbase.dll": ["printf"],
        }
        self.assertEqual(classify(imports), [
            "api-ms-win-crt-heap-l1-1-0.dll",
            "api-ms-win-crt-runtime-l1-1-0.dll",
            "msvcp140.dll",
            "ucrtbase.dll",
            "vcruntime140.dll",
        ])

    def test_windows7_builtin_dlls_are_not_external_crt(self):
        self.assertEqual(classify({
            "kernel32.dll": ["GetProcAddress"],
            "msvcrt.dll": ["malloc"],
            "ole32.dll": ["CoTaskMemFree"],
            "user32.dll": ["GetMessageW"],
        }), [])

    def test_custom_vcruntime_and_msvcp_variants(self):
        self.assertEqual(classify({
            "VCRUNTIME140_1.DLL": ["foo"],
            "MSVCP140_2.DLL": ["bar"],
        }), ["msvcp140_2.dll", "vcruntime140_1.dll"])

    def test_not_broadly_matching_arbitrary_dll_names(self):
        self.assertEqual(classify({"myvcruntime140.dll": [], "api-ms-win-core-foo.dll": []}), [])


if __name__ == "__main__":
    unittest.main()
