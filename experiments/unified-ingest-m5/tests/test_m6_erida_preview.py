"""Never stage or confirm: read-only preview API guard tested over loopback HTTP."""
import json
import re
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from discovery_lab.objects import FileObjectSink
from discovery_lab.unified_pipeline import PageLedger, UnifiedPipeline, NeedsReview, parse_plan
from discovery_lab.erida_export import prepare_erida_batches
from discovery_lab.erida_preview import preview_erida_batches


class PreviewFixture(BaseHTTPRequestHandler):
    requests = []
    fault = None
    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(n)
        self.__class__.requests.append((self.path, self.headers.get("X-API-Key"), body))
        if self.__class__.fault == "redirect":
            self.send_response(302)
            self.send_header("Location", "https://attacker.invalid/steal")
            self.end_headers()
            return
        rows = [int(x) for x in re.findall(r"<h1>Глава (\d+)\.", body.decode("utf-8", "replace"))]
        response = {"kind": "book", "stored": False, "chapter_count": len(rows),
                    "chapters": [{"detected_number": x, "has_text": True} for x in rows], "warnings": []}
        if self.__class__.fault == "wrong_count":
            response["chapter_count"] = 0
        if self.__class__.fault == "warnings":
            response["warnings"] = ["manual mapping needed"]
        data = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)
    def log_message(self, *args):
        pass


class ReadOnlyEridaPreviewTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        folder = Path(__file__).resolve().parents[1] / "examples" / "approved-work"
        plan = parse_plan((folder / "approved-plan.json").read_bytes())
        sink = FileObjectSink(root / "objects")
        result = UnifiedPipeline(PageLedger(root / "state.db"), sink, folder, root / "stage").run(plan)
        self.prepared = prepare_erida_batches(sink, result["handoff_key"], result["handoff_sha256"], root / "out")
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), PreviewFixture)
        self.addCleanup(self.server.server_close)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.server.shutdown)
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        PreviewFixture.requests = []
        PreviewFixture.fault = None

    def call(self):
        return preview_erida_batches(self.prepared["manifest"], erida_origin=self.url, api_key="not-a-real-key")

    def test_read_only_preview_uses_only_expected_route(self):
        report = self.call()
        self.assertEqual(report["status"], "READ_ONLY_PREVIEW_PASSED")
        self.assertFalse(report["stage_created"])
        self.assertEqual(report["batches"][0]["chapters_confirmed_by_preview"], 2)
        self.assertEqual(len(PreviewFixture.requests), 1)
        self.assertEqual(PreviewFixture.requests[0][0], "/v1/imports/preview")
        self.assertEqual(PreviewFixture.requests[0][1], "not-a-real-key")
        self.assertNotIn("not-a-real-key", str(report))
        self.assertNotIn("Second paragraph", str(report))

    def test_invalid_preview_response_fails(self):
        PreviewFixture.fault = "wrong_count"
        with self.assertRaisesRegex(NeedsReview, "disagrees"):
            self.call()
        PreviewFixture.fault = "warnings"
        with self.assertRaisesRegex(NeedsReview, "disagrees"):
            self.call()

    def test_no_redirect_credentials(self):
        PreviewFixture.fault = "redirect"
        with self.assertRaisesRegex(NeedsReview, "redirected"):
            self.call()
        self.assertEqual(len(PreviewFixture.requests), 1)

    def test_http_insecure_remote_refused(self):
        with self.assertRaisesRegex(NeedsReview, "requires HTTPS"):
            preview_erida_batches(self.prepared["manifest"], erida_origin="http://example.com", api_key="key")
        self.assertEqual(PreviewFixture.requests, [])

    def test_corrupt_prepared_batch_blocks_request(self):
        path = Path(self.prepared["manifest"]).parent / self.prepared["batches"][0]["filename"]
        path.write_bytes(path.read_bytes() + b"x")
        with self.assertRaisesRegex(NeedsReview, "modified"):
            self.call()
        self.assertEqual(PreviewFixture.requests, [])

if __name__ == "__main__": unittest.main()

