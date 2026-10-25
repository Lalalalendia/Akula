"""M8 standalone service integration, persistence, fences and HTTP security."""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import shutil
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer
from pathlib import Path

from discovery_lab.service import (AdapterRegistry, IngestWorker, InvalidJob,
                                   JobConflict, JobStore, SavedPageAdapter, WorkerBusy,
                                   make_handler)
from discovery_lab.unified_pipeline import PageLedger, parse_plan

EXAMPLE = Path(__file__).resolve().parents[1] / "examples" / "approved-work"
TOKEN = "m8-local-testing-long-bearer-token"


class M8ServiceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ingest-m8-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.source = self.root / "approved" / "demo-story"
        self.source.mkdir(parents=True)
        self.plan = json.loads((EXAMPLE / "approved-plan.json").read_text())
        for f in EXAMPLE.glob("*.html"):
            shutil.copy2(f, self.source / f.name)
        self.jobs = JobStore(self.root / "state", self.root / "approved")
        self.worker = IngestWorker(self.jobs)

    def submit(self, **kw):
        return self.jobs.submit({"plan": self.plan, **kw})

    def test_queue_survives_restart_and_runs_complete_chain(self):
        job, reused = self.submit(idempotency_key="fixture-1")
        self.assertFalse(reused)
        self.assertEqual(job["status"], "QUEUED")
        # Reopen fresh connections: no in-memory queue or process objects.
        reboot = JobStore(self.root / "state", self.root / "approved")
        result = IngestWorker(reboot).run_once()
        self.assertEqual(result["status"], "COMPLETED", result)
        self.assertEqual(result["result"]["pages_verified"], 3)
        self.assertIn(result["result"]["status"], ("PUBLISHED", "UNCHANGED"))
        self.assertEqual(reboot.get(job["job_id"])["status"], "COMPLETED")
        self.assertIsNone(IngestWorker(reboot).run_once())
        objs = list((self.root / "state" / "objects").rglob("*"))
        self.assertTrue(any(x.is_file() and x.name == "manifest.json" for x in objs))

    def test_duplicate_and_conflicting_plan_fail_closed(self):
        first, _ = self.submit(idempotency_key="key-1")
        again, reused = self.submit(idempotency_key="key-1")
        self.assertTrue(reused)
        self.assertEqual(first["job_id"], again["job_id"])
        with self.assertRaises(JobConflict):
            self.submit(idempotency_key="different-key")
        self.plan["title"] = "Different selected work"
        with self.assertRaises(JobConflict):
            self.submit(idempotency_key="key-1")
        with self.assertRaises(JobConflict):
            self.submit()

    def test_freezes_approved_snapshot_before_queue_run(self):
        job, _ = self.submit()
        source = self.source / "chapter-1-page-1.html"
        source.write_text("<html>changed after submission</html>")
        self.assertEqual(self.worker.run_once()["status"], "COMPLETED")
        snap = self.root / "state" / "approved-snapshots" / job["plan_sha"] / source.name
        self.assertNotEqual(snap.read_bytes(), source.read_bytes())

    def test_crash_claim_and_page_lease_fenced_on_restart(self):
        job, _ = self.submit()
        info, raw, old_token = self.jobs.claim()
        ledger = PageLedger(self.jobs.ledger_path(job["job_id"]))
        plan = parse_plan(raw)
        ledger.register(plan)
        stale = ledger.claim(plan.source_id, "READY")
        self.assertIsNotNone(stale)
        # Simulated killed worker: neither page lease nor queue receipt committed.
        self.assertEqual(IngestWorker(JobStore(self.root / "state", self.root / "approved")).run_once()["status"], "COMPLETED")
        with self.assertRaises(JobConflict):
            self.jobs.finish(job["job_id"], old_token, result={"forged": True})
        with self.assertRaises(Exception):
            ledger.commit(stale, phase="RAW", key="raw/x/evil.html", sha="0"*64)

    def test_cancel_and_explicit_retry(self):
        job, _ = self.submit()
        self.assertEqual(self.jobs.cancel(job["job_id"])["status"], "CANCELLED")
        self.assertIsNone(self.worker.run_once())
        self.assertEqual(self.jobs.retry(job["job_id"])["status"], "QUEUED")
        self.assertEqual(self.worker.run_once()["status"], "COMPLETED")
        with self.assertRaises(JobConflict):
            self.jobs.cancel(job["job_id"])

    def test_failed_content_is_blocked_from_blind_retry(self):
        job, _ = self.submit()
        approved = self.root / "state" / "approved-snapshots" / job["plan_sha"]
        (approved / "chapter-1-page-1.html").unlink()
        result = self.worker.run_once()
        self.assertEqual(result["status"], "NEEDS_REVIEW")
        self.assertEqual(result["error_code"], "NeedsReview")
        with self.assertRaises(JobConflict):
            self.jobs.retry(job["job_id"])

    def test_concurrent_idempotent_submission_one_job(self):
        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            futures = [pool.submit(self.submit) for _ in range(12)]
            ids = {f.result()[0]["job_id"] for f in futures}
        self.assertEqual(len(ids), 1)
        self.assertEqual(len(self.jobs.list()), 1)

    def test_rejects_source_path_and_unknown_profile(self):
        self.plan["chapters"][0]["pages"][0]["fixture_file"] = "../../etc/passwd"
        with self.assertRaises(InvalidJob):
            self.submit()
        self.plan = json.loads((EXAMPLE / "approved-plan.json").read_text())
        self.plan["parser_profile"] = "shell-runner-v1"
        with self.assertRaises(InvalidJob):
            self.submit()


    def test_one_worker_exclusive_lock(self):
        self.submit()
        other = IngestWorker(JobStore(self.root / "state", self.root / "approved"))
        with self.worker._exclusive():
            with self.assertRaises(WorkerBusy):
                other.run_once()
        self.assertEqual(other.run_once()["status"], "COMPLETED")

    def test_preloaded_raw_snapshots_reuse_existing_immutable_sink(self):
        from discovery_lab.objects import FileObjectSink
        store = FileObjectSink(self.root / "state" / "objects")
        for ch in self.plan["chapters"]:
            for page in ch["pages"]:
                name = page.pop("fixture_file")
                data = (self.source / name).read_bytes()
                key = f"raw/demo-story/preloaded/{name}"
                page["raw_key"] = key
                page["raw_sha256"] = store.put_verified(key, data)
        job, _ = self.submit()
        # No source directory is needed for a preloaded RAW-object plan.
        shutil.rmtree(self.source)
        output = self.worker.run_once()
        self.assertEqual(output["status"], "COMPLETED", output)
        self.assertEqual(output["result"]["pages_verified"], 3)

    def test_registry_rejects_duplicate_profile(self):
        r = AdapterRegistry()
        r.register(SavedPageAdapter("fixture-chapter-html-v1"))
        with self.assertRaises(ValueError):
            r.register(SavedPageAdapter("fixture-chapter-html-v1"))

    def test_http_auth_enqueue_status_cancel_and_api_not_staging(self):
        srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(self.jobs, TOKEN))
        thread = threading.Thread(target=srv.serve_forever, kwargs={"poll_interval": .03}, daemon=True)
        thread.start()
        self.addCleanup(lambda: (srv.shutdown(), srv.server_close(), thread.join(timeout=3)))
        base = f"http://127.0.0.1:{srv.server_port}"

        def request(method, path, content=None, bearer=True):
            payload = json.dumps(content).encode() if content is not None else None
            headers = {"Content-Type": "application/json"}
            if bearer:
                headers["Authorization"] = "Bearer " + TOKEN
            req = urllib.request.Request(base + path, method=method, data=payload, headers=headers)
            try:
                with urllib.request.urlopen(req, timeout=5) as r:
                    return r.status, json.loads(r.read())
            except urllib.error.HTTPError as e:
                return e.code, json.loads(e.read())

        self.assertEqual(request("GET", "/healthz", bearer=False)[0], 200)
        self.assertEqual(request("GET", "/v1/jobs", bearer=False)[0], 401)
        self.assertEqual(request("POST", "/v1/jobs", {"plan": self.plan}, bearer=False)[0], 401)
        self.assertEqual(request("GET", "/v1/adapters")[1]["live_crawling"], False)
        code, body = request("POST", "/v1/jobs", {"plan": self.plan})
        self.assertEqual(code, 201, body)
        jid = body["job"]["job_id"]
        self.assertNotIn("plan", body["job"])
        self.assertEqual(request("POST", "/v1/jobs", {"plan": self.plan})[0], 200)
        self.assertEqual(request("GET", f"/v1/jobs/{jid}")[1]["status"], "QUEUED")
        self.assertEqual(request("POST", "/v1/imports/stages", {})[0], 404)
        self.assertEqual(request("POST", f"/v1/jobs/{jid}/cancel", {})[0], 400)
        self.assertEqual(request("POST", f"/v1/jobs/{jid}/cancel")[0], 200)
        self.assertEqual(request("POST", f"/v1/jobs/{jid}/retry")[0], 200)
        self.assertEqual(self.worker.run_once()["status"], "COMPLETED")
        self.assertEqual(request("GET", f"/v1/jobs/{jid}")[1]["status"], "COMPLETED")
        self.assertEqual(request("GET", "/v1/jobs?any=1")[0], 400)


if __name__ == "__main__":
    unittest.main()
