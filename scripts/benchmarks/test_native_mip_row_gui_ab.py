#!/usr/bin/env python3
"""Pure-stdlib preflight for the real-window RGBA A/B harness.

The tests run BEFORE Cargo compilation on each OS. They specifically guard
against the missing note digest that previously wasted both full GUI builds.
"""
from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from native_mip_row_gui_ab import validate_fixture_hashes


class FixtureParity(unittest.TestCase):
    PNG = "a" * 64
    NOTE = "b" * 64

    def row(self, *, image=None, note=None):
        return {
            "image_sha256": image if image is not None else self.PNG,
            "note_sha256": note if note is not None else self.NOTE,
        }

    def test_first_and_subsequent_trials_keep_note_digest(self):
        first = validate_fixture_hashes(self.row(), self.PNG, None)
        self.assertEqual(first, self.NOTE)
        for _ in range(5):
            first = validate_fixture_hashes(self.row(), self.PNG, first)
        self.assertEqual(first, self.NOTE)

    def test_changed_image_fails_before_result_reporting(self):
        with self.assertRaisesRegex(ValueError, "PNG"):
            validate_fixture_hashes(self.row(image="c" * 64), self.PNG, self.NOTE)

    def test_changed_markdown_fails_before_result_reporting(self):
        with self.assertRaisesRegex(ValueError, "Note bytes"):
            validate_fixture_hashes(self.row(note="d" * 64), self.PNG, self.NOTE)

    def test_missing_markdown_field_fails_as_validation_error(self):
        row = self.row()
        row.pop("note_sha256")
        with self.assertRaisesRegex(ValueError, "Missing/invalid Markdown"):
            validate_fixture_hashes(row, self.PNG, self.NOTE)


if __name__ == "__main__":
    unittest.main()
