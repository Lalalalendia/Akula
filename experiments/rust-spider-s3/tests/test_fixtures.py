import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from fixtures import generate


class TestFixture(unittest.TestCase):
    def test_reproducible(self):
        with tempfile.TemporaryDirectory() as a, tempfile.TemporaryDirectory() as b:
            first = generate(Path(a), 5, 1024)
            second = generate(Path(b), 5, 1024)
            self.assertEqual(first, second)
            self.assertEqual(5, len(first))
            for name, row in first.items():
                self.assertEqual(row["sha256"], hashlib.sha256((Path(a) / "pages" / name).read_bytes()).hexdigest())
                self.assertEqual((Path(a) / "pages" / name).read_bytes(), (Path(b) / "pages" / name).read_bytes())
            self.assertEqual(first, json.loads((Path(a) / "manifest.json").read_text()))
            self.assertEqual(5, (Path(a) / "index.html").read_text().count("<a href"))
