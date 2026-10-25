"""M6 cross-product contract: immutable selected source -> Erida import HTML."""
import hashlib
import json
import re
import tempfile
import unittest
from pathlib import Path

from discovery_lab.objects import FileObjectSink
from discovery_lab.unified_pipeline import PageLedger, UnifiedPipeline, NeedsReview, parse_plan
from discovery_lab.erida_export import prepare_erida_batches, verify_handoff


class EridaExportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        source = Path(__file__).resolve().parents[1] / "examples" / "approved-work"
        self.plan = parse_plan((source / "approved-plan.json").read_bytes())
        self.sink = FileObjectSink(self.root / "objects")
        self.pipe = UnifiedPipeline(PageLedger(self.root / "state.db"), self.sink,
                                    source, self.root / "stage")
        self.result = self.pipe.run(self.plan)
        self.out = self.root / "ready-for-erida"

    def export(self, limit=1024 * 1024):
        return prepare_erida_batches(self.sink, self.result["handoff_key"],
                                     self.result["handoff_sha256"], self.out, max_batch_bytes=limit)

    def test_m5_to_m6_readonly_and_idempotent(self):
        obj = self.export()
        self.assertEqual(obj["status"], "PREPARED_NOT_STAGED")
        self.assertTrue(obj["requires_actor_confirmation"])
        self.assertFalse(obj["stage_automatically"])
        self.assertEqual(obj["expected_chapters"], 2)
        self.assertEqual(len(obj["batches"]), 1)
        content = (self.out / obj["batches"][0]["filename"]).read_text()
        self.assertIn("<h1>Глава 1. Chapter 1</h1>", content)
        self.assertIn("<h1>Глава 2. Chapter 2</h1>", content)
        self.assertEqual(re.findall(r"<h1>Глава (\d+)\.", content), ["1", "2"])
        self.assertEqual(self.export()["manifest"], obj["manifest"])

    def test_bounded_batch_split(self):
        first = self.export()
        body = (self.out / first["batches"][0]["filename"]).read_bytes()
        first_heading = body.find(b"<h1>")
        second_heading = body.find(b"<h1>", first_heading + 1)
        limit = len(body[:second_heading]) + 20
        self.assertGreaterEqual(limit, 250)
        d = prepare_erida_batches(self.sink, self.result["handoff_key"],
                                  self.result["handoff_sha256"], self.root / "split-output",
                                  max_batch_bytes=limit)
        self.assertEqual(len(d["batches"]), 2)
        self.assertEqual([r["chapter_count"] for r in d["batches"]], [1,1])
        self.assertEqual([r["first_ordinal"] for r in d["batches"]], [1,2])

    def test_tampered_handoff_rejected(self):
        key = self.result["handoff_key"]
        path = self.sink._path(key)
        raw = path.read_bytes(); path.write_bytes(raw+b" ")
        with self.assertRaisesRegex(NeedsReview, "SHA-256"):
            self.export()

    def test_invalid_handoff_flags_rejected_with_matching_digest(self):
        key = self.result["handoff_key"]
        path = self.sink._path(key)
        j = json.loads(path.read_bytes()); j["publication_permitted"] = True
        body = json.dumps(j, ensure_ascii=False).encode(); path.write_bytes(body)
        with self.assertRaisesRegex(NeedsReview, "read-only"):
            prepare_erida_batches(self.sink, key, hashlib.sha256(body).hexdigest(), self.out)

    def test_corrupt_chapter_rejected_before_any_export(self):
        chapter = self.sink._path(f'works/{self.plan.source_id}/versions/{self.result["revision_id"]}/chapters/0001.md')
        chapter.write_bytes(chapter.read_bytes()+b'x')
        with self.assertRaisesRegex(NeedsReview, "corrupted"):
            self.export()
        self.assertFalse(self.out.exists())

    def test_missing_raw_rejected_before_export(self):
        obj = json.loads(self.sink.get(self.result["handoff_key"]))
        self.sink._path(obj["items"][0]["raw_key"]).unlink()
        with self.assertRaises(FileNotFoundError):
            self.export()
        self.assertFalse(self.out.exists())

    def test_conflicting_output_never_overwritten(self):
        meta = self.export()
        file = self.out / meta["batches"][0]["filename"]
        file.write_text("existing newer personal edit", encoding="utf8")
        with self.assertRaisesRegex(NeedsReview, "conflicts"):
            self.export()
        self.assertEqual(file.read_text(), "existing newer personal edit")

    def test_reject_size_and_scope(self):
        with self.assertRaises(ValueError):
            self.export(150)
        with self.assertRaises(ValueError):
            self.export(13 * 1024 * 1024)
        proof, chapters = verify_handoff(self.sink, self.result["handoff_key"], self.result["handoff_sha256"])
        self.assertEqual(len(chapters), 2)
        self.assertEqual(proof["scope"], "selected_manifest_only")

if __name__ == "__main__":
    unittest.main()

