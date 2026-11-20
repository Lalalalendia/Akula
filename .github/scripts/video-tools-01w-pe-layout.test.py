#!/usr/bin/env python3
"""01w synthetic PE negative/positive tests; no wheel download or PE execution."""
import importlib.util
import struct
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("video_tools_01w", HERE / "video-tools-01w-pe-layout.py")
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)

def make_pe(timestamp=1, checksum=2, code=0x50, virtual_size=0x200):
    image = bytearray(0x400)
    image[:2] = b"MZ"
    struct.pack_into("<I", image, 0x3c, 0x80)
    image[0x80:0x84] = b"PE\x00\x00"
    struct.pack_into("<HHIIIHH", image, 0x84, 0x8664, 1,
                     timestamp, 0, 0, 0xf0, 0x2022)
    opt = 0x98
    struct.pack_into("<H", image, opt, 0x20b)
    struct.pack_into("<I", image, opt + 56, 0x2000)
    struct.pack_into("<I", image, opt + 60, 0x200)
    struct.pack_into("<I", image, opt + 64, checksum)
    struct.pack_into("<H", image, opt + 68, 3)
    struct.pack_into("<8sIIIIIIHHI", image, opt + 0xf0,
                     b".text\x00\x00\x00", virtual_size,
                     0x1000, 0x200, 0x200, 0, 0, 0, 0, 0x60000020)
    image[0x200:0x400] = bytes([code]) * 0x200
    return bytes(image)

class PeLayoutTests(unittest.TestCase):
    def test_exact_pe_parser_and_section(self):
        result = M.parse_pe(make_pe())
        self.assertEqual(result["machine"], 0x8664)
        self.assertEqual(result["size"], 0x400)
        self.assertEqual(len(result["sections"]), 1)
        self.assertEqual(result["sections"][0]["name"], ".text")

    def test_timestamp_and_checksum_do_not_hide_real_section_change(self):
        a = make_pe(timestamp=1, checksum=2)
        b = make_pe(timestamp=3, checksum=4)
        report = M.compare_pe(a, b)
        self.assertFalse(report["byte_identical"])
        self.assertTrue(report["only_timestamp_checksum_may_differ"])
        self.assertEqual(report["difference_class"], "ONLY_COFF_TIMESTAMP_AND_CHECKSUM_DIFFER")
        self.assertEqual(report["matching_raw_sections"], 1)
        changed = M.compare_pe(a, make_pe(timestamp=3, checksum=4, code=0x51))
        self.assertFalse(changed["only_timestamp_checksum_may_differ"])
        self.assertEqual(changed["difference_class"], "SECTION_CONTENT_OR_LAYOUT_DIFFERS")
        self.assertEqual(changed["matching_raw_sections"], 0)
        self.assertEqual(M.compare_pe(a, a)["difference_class"], "BYTE_IDENTICAL")

    def test_layout_changes_are_detected(self):
        changed = M.compare_pe(make_pe(), make_pe(virtual_size=0x100))
        self.assertFalse(changed["section_layout_identical"])
        self.assertEqual(changed["difference_class"], "SECTION_CONTENT_OR_LAYOUT_DIFFERS")

    def test_bounded_parser_rejects_non_pe_truncation_wrong_machine_and_invalid_sections(self):
        with self.assertRaisesRegex(M.Hold, "DOS MZ"):
            M.parse_pe(b"not a PE")
        a = bytearray(make_pe())
        a[0x80:0x84] = b"WRNG"
        with self.assertRaisesRegex(M.Hold, "signature"):
            M.parse_pe(bytes(a))
        b = bytearray(make_pe())
        struct.pack_into("<H", b, 0x84, 0x14c)
        with self.assertRaisesRegex(M.Hold, "PE32"):
            M.parse_pe(bytes(b))
        c = bytearray(make_pe())
        struct.pack_into("<I", c, 0x188 + 20, 0xfffff000)
        with self.assertRaisesRegex(M.Hold, "raw bounds"):
            M.parse_pe(bytes(c))

    def test_nonrelease_flags_and_pinned_member_names(self):
        self.assertEqual(len(M.BOOTLOADER_NAMES), 4)
        self.assertEqual(set(M.BOOTLOADER_NAMES),
                         {"run.exe", "run_d.exe", "runw.exe", "runw_d.exe"})
        report = M.V.base_report("a" * 40, "01w")
        for key in ("release_admitted", "legal_approval",
                    "complete_corresponding_source_available",
                    "publisher_signing_authorized",
                    "corresponding_source_rebuilt_and_verified"):
            self.assertIs(report[key], False)

if __name__ == "__main__":
    unittest.main()
