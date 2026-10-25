"""M7 stages only when operator requests it; never commits imported chapters."""
import hashlib
import json
import re
import tempfile
import threading
import time
import unittest
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from discovery_lab.erida_export import prepare_erida_batches
from discovery_lab.erida_stage import stage_erida_batches
from discovery_lab.objects import FileObjectSink
from discovery_lab.unified_pipeline import NeedsReview, PageLedger, UnifiedPipeline, parse_plan


class FakeErida(BaseHTTPRequestHandler):
    events = []
    staged = {}
    mode = "ok"

    def do_POST(self):
        n = int(self.headers.get("Content-Length", "0"))
        buf = self.rfile.read(n)
        self.__class__.events.append((self.path, self.headers.get("Idempotency-Key"),
                                      self.headers.get("X-API-Key")))
        if self.path not in ("/v1/imports/stages", "/v1/imports/preview"):
            self.send_error(404)
            return
        if self.__class__.mode == "preview_bad" and self.path.endswith("/preview"):
            preview = self._preview(buf)
            preview["warnings"] = ["unassigned"]
            return self._send(preview, 200)
        if self.__class__.mode == "stage_fail" and self.path.endswith("/stages"):
            self.send_error(503)
            return
        preview = self._preview(buf)
        if self.path.endswith("/preview"):
            return self._send(preview, 200)
        filename = re.search(rb'filename="([^"]+)"', buf).group(1).decode()
        payload = buf.split(b"\r\n\r\n", 1)[1].rsplit(b"\r\n--M7-", 1)[0]
        key = self.headers["Idempotency-Key"]
        if key not in self.__class__.staged:
            self.__class__.staged[key] = {"stage_id": str(uuid.uuid4()), "status": "staged",
                "file_sha256": hashlib.sha256(payload).hexdigest(),
                "expires_at": time.time() + 1800, "unassigned_prefix_saved": False,
                "preview": preview}
        return self._send(self.__class__.staged[key], 201)

    @staticmethod
    def _preview(data):
        nums = [int(x) for x in re.findall(rb"<h1>\s*\xd0\x93\xd0\xbb\xd0\xb0\xd0\xb2\xd0\xb0\s+(\d+)\.", data)]
        return {"kind": "book", "stored": False, "chapter_count": len(nums),
                "chapters": [{"detected_number": n, "has_text": True} for n in nums],
                "warnings": []}

    def _send(self, data, code):
        raw = json.dumps(data).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def log_message(self, *a):
        pass


class StageTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        folder = Path(__file__).resolve().parents[1] / "examples" / "approved-work"
        plan = parse_plan((folder / "approved-plan.json").read_bytes())
        sink = FileObjectSink(root / "objects")
        result = UnifiedPipeline(PageLedger(root / "stage.db"), sink, folder, root / "stage").run(plan)
        self.prepared = prepare_erida_batches(sink, result["handoff_key"], result["handoff_sha256"], root / "out")
        FakeErida.events = []
        FakeErida.staged = {}
        FakeErida.mode = "ok"
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), FakeErida)
        th = threading.Thread(target=self.server.serve_forever, daemon=True)
        th.start()
        self.addCleanup(self.server.shutdown)
        self.addCleanup(self.server.server_close)
        self.origin = f"http://127.0.0.1:{self.server.server_port}"

    def stage(self, **kwargs):
        return stage_erida_batches(self.prepared["manifest"],
                                   erida_origin=self.origin, api_key="fake", **kwargs)

    def test_no_write_without_explicit_optin(self):
        with self.assertRaisesRegex(NeedsReview, "explicit"):
            self.stage()
        self.assertEqual(FakeErida.events, [])

    def test_stage_only_and_idempotency(self):
        a = self.stage(allow_stage=True)
        b = self.stage(allow_stage=True)
        self.assertEqual(a["status"], "STAGED_AWAITING_ACTOR_CONFIRMATION")
        self.assertFalse(a["confirmed"])
        self.assertFalse(a["published"])
        self.assertEqual(a["stages"][0]["stage_id"], b["stages"][0]["stage_id"])
        self.assertEqual([p for p, _, _ in FakeErida.events],
                         ["/v1/imports/preview", "/v1/imports/stages"] * 2)
        self.assertEqual(len(FakeErida.staged), 1)
        self.assertNotIn("fake", str(a))
        self.assertNotIn("Original synthetic prose", str(a))

    def test_warning_on_preview_prevents_stage(self):
        FakeErida.mode = "preview_bad"
        with self.assertRaisesRegex(NeedsReview, "disagrees"):
            self.stage(allow_stage=True)
        self.assertEqual([p for p, _, _ in FakeErida.events], ["/v1/imports/preview"])

    def test_stage_transport_error_is_partial_without_confirm(self):
        FakeErida.mode = "stage_fail"
        result = self.stage(allow_stage=True)
        self.assertEqual(result["status"], "PARTIAL_STAGE_NEEDS_REVIEW")
        self.assertEqual(result["stages"], [])
        self.assertNotIn("/confirm", str(FakeErida.events))

    def test_corrupt_batch_rejected_before_network(self):
        file = Path(self.prepared["manifest"]).parent / self.prepared["batches"][0]["filename"]
        file.write_bytes(file.read_bytes() + b"bad")
        with self.assertRaisesRegex(NeedsReview, "modified|missing"):
            self.stage(allow_stage=True)
        self.assertEqual(FakeErida.events, [])

    def test_quota_or_order_violation_rejected_before_network(self):
        p = Path(self.prepared["manifest"])
        j = json.loads(p.read_bytes())
        j["batches"] *= 4
        p.write_text(json.dumps(j))
        with self.assertRaisesRegex(NeedsReview, "three unconfirmed"):
            self.stage(allow_stage=True)
        self.assertEqual(FakeErida.events, [])

    def test_insecure_remote_origin_rejected(self):
        with self.assertRaisesRegex(NeedsReview, "requires HTTPS"):
            stage_erida_batches(self.prepared["manifest"], erida_origin="http://example.com",
                                api_key="fake", allow_stage=True)
        self.assertEqual(FakeErida.events, [])


if __name__ == "__main__":
    unittest.main()
