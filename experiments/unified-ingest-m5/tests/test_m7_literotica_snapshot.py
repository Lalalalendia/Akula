"""Saved-page-only Literotica parser and full M5 RAW→parser→publisher handoff."""
import json
import tempfile
import unittest
from pathlib import Path

from discovery_lab.erida_export import prepare_erida_batches
from discovery_lab.literotica_snapshot import SnapshotRejected, parse_saved_story_page, story_url, series_url
from discovery_lab.objects import FileObjectSink
from discovery_lab.unified_pipeline import NeedsReview, PageLedger, UnifiedPipeline, parse_plan


def html_page(number: int, total: int, *, duplicate=False, body=True, observed=None) -> bytes:
    text = ("This is original synthetic story text for a rights-cleared local fixture "
            f"with multiple complete paragraphs on distinct page {number} of {total}.")
    area = f'<div class="_article__content_4z2"><div><p>{text}</p><p>Next original line<br>after a break.</p></div></div>'
    if duplicate:
        area += area
    if not body:
        area = '<div class="_unknown_html_4z2"><p>Text exists but selector changed.</p></div>'
    shown = observed or (number, total)
    return (f'<html><body>{area}'
            f'<nav class="panel _pagination_abc"><a href="?page=1">1</a>'
            f'<a href="?page={total}">{total}</a></nav>'
            f'<span>Page {shown[0]} of {shown[1]}</span></body></html>').encode()


class StoryParserTests(unittest.TestCase):
    def test_identifies_story_page_without_dropping_page_number(self):
        self.assertEqual(story_url("https://www.literotica.com/s/fictional-work?page=2").page, 2)
        self.assertEqual(story_url("https://www.literotica.com/s/fictional-work").page, 1)
        self.assertEqual(series_url("https://www.literotica.com/series/se/fictional-series")[1], "fictional-series")
        for unsafe in (
            "http://www.literotica.com/s/fictional-work", "https://www.literotica.com.evil.invalid/s/x",
            "https://www.literotica.com/s/fictional-work?page=2&page=3",
            "https://www.literotica.com/s/fictional-work?sort=popular",
            "https://www.literotica.com/s/fictional-work/comments",
            "https://www.literotica.com/s/fictional-work?page=0",
        ):
            with self.subTest(url=unsafe), self.assertRaises((SnapshotRejected, ValueError)):
                story_url(unsafe)

    def test_parses_only_approved_saved_text_and_pagination(self):
        url = "https://www.literotica.com/s/fictional-work?page=2"
        doc = parse_saved_story_page(html_page(2, 2), url=url, page=2, expected_pages=2)
        self.assertIn("rights-cleared local fixture", doc.text)
        self.assertIn("Next original line\nafter a break", doc.text)
        self.assertEqual(doc.layout, "article-content-2026")
        for payload in (html_page(2, 2, duplicate=True), html_page(2, 2, body=False),
                        html_page(2, 2, observed=(2,3))):
            with self.subTest(data=payload[:70]), self.assertRaises(SnapshotRejected):
                parse_saved_story_page(payload, url=url, page=2, expected_pages=2)

    def test_page_count_fail_closed(self):
        with self.assertRaises(SnapshotRejected):
            parse_saved_story_page(html_page(1, 2), url="https://www.literotica.com/s/fictional-work",
                                   page=1, expected_pages=1)


class PipelineLiteroticaTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)
        self.fixtures = self.root / "approved"
        self.fixtures.mkdir()
        (self.fixtures / "part-1.html").write_bytes(html_page(1, 2))
        (self.fixtures / "part-2.html").write_bytes(html_page(2, 2))
        self.document = {
            "schema": "selected-work-v1", "source_id": "story-synthetic-m7",
            "title": "Fictional story", "author": "Example Author",
            "source_url": "https://www.literotica.com/series/se/fictional-series",
            "language": "en", "parser_profile": "literotica-saved-story-v1",
            "tags": ["source/literotica"], "source_tags": ["Original Tag"],
            "source_categories": ["Fictional Category"],
            "chapters": [{"ordinal": 1, "title": "Part One", "pages": [
                {"number": 1, "source_url": "https://www.literotica.com/s/fictional-work",
                 "fixture_file": "part-1.html"},
                {"number": 2, "source_url": "https://www.literotica.com/s/fictional-work?page=2",
                 "fixture_file": "part-2.html"}]}]
        }
        self.sink = FileObjectSink(self.root / "objects")
        self.pipeline = UnifiedPipeline(PageLedger(self.root / "state.db"), self.sink,
                                        self.fixtures, self.root / "stage")

    def plan(self):
        return parse_plan(json.dumps(self.document).encode())

    def test_full_offline_pipeline_verified_handoff_and_erida_html(self):
        result = self.pipeline.run(self.plan())
        self.assertEqual(result["pages_verified"], 2)
        self.assertEqual(result["status"], "PUBLISHED")
        proof = json.loads(self.sink.get(result["handoff_key"]))
        self.assertEqual(proof["parser_profile"], "literotica-saved-story-v1")
        metadata = json.loads(self.sink.get(
            f'works/story-synthetic-m7/versions/{result["revision_id"]}/metadata.json'))
        self.assertEqual(metadata["coverage_scope"], "selected_manifest_only")
        self.assertEqual(metadata["chapters"][0]["pages_verified"], 2)
        prepared = prepare_erida_batches(self.sink, result["handoff_key"],
                                         result["handoff_sha256"], self.root / "erida")
        self.assertEqual(prepared["expected_chapters"], 1)
        self.assertEqual(prepared["status"], "PREPARED_NOT_STAGED")
        self.assertEqual(self.pipeline.run(self.plan())["status"], "UNCHANGED")

    def test_fails_before_publish_on_missing_or_mixed_page(self):
        self.document["chapters"][0]["pages"][1]["source_url"] = "https://www.literotica.com/s/another-story?page=2"
        with self.assertRaisesRegex(NeedsReview, "different story"):
            self.plan()

    def test_html_changed_to_unrecognized_layout_prevents_publication(self):
        (self.fixtures / "part-1.html").write_bytes(html_page(1, 2, body=False))
        with self.assertRaisesRegex(NeedsReview, "needs parser review"):
            self.pipeline.run(self.plan())
        self.assertFalse(self.sink._path("works/story-synthetic-m7/current.json").exists())


if __name__ == "__main__":
    unittest.main()
