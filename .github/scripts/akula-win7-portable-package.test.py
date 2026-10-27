#!/usr/bin/env python3
"""Exact COM import decisions for one Win7-compatible Akula EXE.

Real CI additionally performs a full Win7-target build, dumpbin PE audit,
staging, second PE audit, NSIS packaging and installed GUI/PDF smoke.
"""
from pathlib import Path
import runpy
import unittest

package = runpy.run_path(str(Path(__file__).with_name("akula-win7-portable-package.py")))
decision = package["com_rewrite_offset"]
EXPECTED = ["CoCreateFreeThreadedMarshaler", "CoTaskMemFree"]


class Win7ComImportTests(unittest.TestCase):
    def test_precompatible_ole32_does_not_rewrite_any_bytes(self):
        report = {"imports": {"ole32.dll": [*EXPECTED, "CoCreateInstance"]}}
        descriptors = [(100, b"ole32.dll"), (200, b"user32.dll")]
        self.assertIsNone(decision(report, descriptors))

    def test_legacy_exact_two_combase_exports_are_remappable(self):
        report = {"imports": {"combase.dll": EXPECTED, "ole32.dll": ["CoInitializeEx"]}}
        descriptors = [(256, b"combase.dll"), (512, b"ole32.dll")]
        self.assertEqual(decision(report, descriptors), 256)

    def test_extra_combase_export_is_rejected(self):
        report = {"imports": {"combase.dll": [*EXPECTED, "RoOriginateErrorW"]}}
        with self.assertRaises(SystemExit):
            decision(report, [(50, b"combase.dll")])

    def test_combase_descriptor_disagrees_with_audit(self):
        report = {"imports": {"ole32.dll": EXPECTED}}
        with self.assertRaises(SystemExit):
            decision(report, [(50, b"combase.dll"), (90, b"ole32.dll")])

    def test_combase_report_without_descriptor_is_rejected(self):
        report = {"imports": {"combase.dll": EXPECTED, "ole32.dll": EXPECTED}}
        with self.assertRaises(SystemExit):
            decision(report, [(90, b"ole32.dll")])

    def test_duplicate_combase_descriptors_are_rejected(self):
        report = {"imports": {"combase.dll": EXPECTED}}
        with self.assertRaises(SystemExit):
            decision(report, [(50, b"combase.dll"), (80, b"combase.dll")])

    def test_missing_required_ole32_export_is_rejected(self):
        report = {"imports": {"ole32.dll": [EXPECTED[0]]}}
        with self.assertRaises(SystemExit):
            decision(report, [(80, b"ole32.dll")])

    def test_missing_ole32_descriptor_is_rejected(self):
        report = {"imports": {"ole32.dll": EXPECTED}}
        with self.assertRaises(SystemExit):
            decision(report, [(80, b"kernel32.dll")])

    def test_invalid_audit_import_map_is_rejected(self):
        with self.assertRaises(SystemExit):
            decision({"imports": None}, [(80, b"ole32.dll")])


if __name__ == "__main__":
    unittest.main()
