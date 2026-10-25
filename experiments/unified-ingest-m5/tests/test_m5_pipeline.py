"""M5: executable end-to-end RAW->parser->M4 streaming publisher->handoff."""
import hashlib
import json
import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from discovery_lab.objects import FileObjectSink, ObjectConflict
from discovery_lab.publisher import IncompleteWork
from discovery_lab.unified_pipeline import (NeedsReview, InjectCrash, PageLedger,
                                             UnifiedPipeline, parse_plan, parse_snapshot)


class PipelineFixtureTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.fixtures = self.root / "fixtures"
        self.fixtures.mkdir()
        self.content = (
            '<html><article data-fixture-work="work-demo" data-chapter="1" '
            'data-page="1"><p>Original synthetic prose A.</p>'
            '<p>Original synthetic prose B.</p></article></html>'
        ).encode()
        (self.fixtures / "chapter1.html").write_bytes(self.content)
        self.doc = {"schema": "selected-work-v1", "source_id": "work-demo",
                    "title": "A test book", "author": "Fixture Writer",
                    "source_url": "https://fixture.invalid/s/work-demo", "language": "en",
                    "tags": ["source/lab"], "source_tags": ["Sample"],
                    "source_categories": ["Synthetic"], "chapters": [{"ordinal": 1,
                    "title": "Chapter 1", "pages": [{"number": 1,
                    "source_url": "https://fixture.invalid/s/chapter1?page=1",
                    "fixture_file": "chapter1.html"}]}]}
        self.plan = parse_plan(self.data())
        self.sink = FileObjectSink(self.root / "objects")
        self.ledger = PageLedger(self.root / "state.sqlite3")
        self.pipe = UnifiedPipeline(self.ledger, self.sink, self.fixtures, self.root / "stage")

    def data(self):
        return json.dumps(self.doc, sort_keys=True, ensure_ascii=False).encode()

    def tearDown(self):
        self.tmp.cleanup()

    def test_complete_two_phase_publication_and_readonly_handoff(self):
        report = self.pipe.run(self.plan)
        self.assertEqual(report["status"], "PUBLISHED")
        self.assertEqual(report["pages_verified"], 1)
        self.assertEqual(self.ledger.status("work-demo")["states"], {"PARSED": 1})
        proof = json.loads(self.sink.get(report["handoff_key"]))
        self.assertEqual(proof["schema"], "verified-source-handoff-v1")
        self.assertTrue(proof["requires_actor_confirmation"])
        self.assertFalse(proof["publication_permitted"])
        self.assertEqual(proof["items"][0]["raw_sha256"], hashlib.sha256(self.content).hexdigest())
        self.assertIn(b"Original synthetic prose A.",
                      self.sink.get(f"works/work-demo/versions/{report['revision_id']}/story.md"))
        self.assertEqual(report["fetched"], 1)
        self.assertEqual(report["parsed"], 1)
        self.assertEqual(self.pipe.run(self.plan)["status"], "UNCHANGED")
        self.assertEqual(len([p for p in self.sink.keys() if p.endswith("current.json")]), 1)
        with self.ledger.conn() as db:
            self.assertEqual(db.execute("SELECT COUNT(*) FROM stage_events").fetchone()[0], 2)

    def test_interruption_raw_put_before_sql_receipt_resumes_without_refetch(self):
        self.ledger.register(self.plan)
        with self.assertRaises(InjectCrash):
            self.pipe.fetch(self.plan, crash_after_raw_put=True)
        raw = [x for x in self.sink.keys() if x.startswith("raw/")]
        self.assertEqual(len(raw), 1)
        # Expire only the abandoned lease; real workers use time-based recovery.
        with self.ledger.tx() as db:
            db.execute("UPDATE source_pages SET lease_until=0 WHERE work_id='work-demo'")
        self.assertEqual(self.pipe.run(self.plan)["status"], "PUBLISHED")
        self.assertEqual(len([x for x in self.sink.keys() if x.startswith("raw/")]), 1)

    def test_parse_replay_without_network_or_fixtures(self):
        self.ledger.register(self.plan)
        report = self.pipe.run(self.plan, stop_after="fetch")
        self.assertEqual(report["status"], "RAW_READY")
        (self.fixtures / "chapter1.html").unlink()
        self.assertEqual(self.pipe.run(self.plan)["status"], "PUBLISHED")

    def test_unrecognized_html_fails_no_current(self):
        (self.fixtures / "chapter1.html").write_text('<article><p>Unknown schema</p></article>')
        with self.assertRaises(NeedsReview):
            self.pipe.run(self.plan)
        self.assertFalse(any(x.startswith("works/") for x in self.sink.keys()))
        self.assertEqual(self.ledger.status("work-demo")["states"], {"NEEDS_REVIEW": 1})

    def test_raw_corruption_before_parse_fails_closed(self):
        self.ledger.register(self.plan)
        self.pipe.fetch(self.plan)
        key = self.ledger.rows("work-demo")[0]["raw_key"]
        self.sink._path(key).write_bytes(b"corrupt")
        with self.assertRaises(ObjectConflict):
            self.pipe.parse(self.plan)
        self.assertFalse(any(x.startswith("works/") for x in self.sink.keys()))

    def test_existing_raw_from_rust_transport(self):
        key = "raw/work-demo/approved/chapter1.html"
        self.sink.put_verified(key, self.content)
        self.doc["chapters"][0]["pages"][0].pop("fixture_file")
        self.doc["chapters"][0]["pages"][0].update(raw_key=key,raw_sha256=hashlib.sha256(self.content).hexdigest())
        plan = parse_plan(self.data())
        self.fixtures.rmdir() if not any(self.fixtures.iterdir()) else (self.fixtures/"chapter1.html").unlink()
        self.assertEqual(self.pipe.run(plan)["status"], "PUBLISHED")
        self.assertEqual(self.ledger.rows("work-demo")[0]["raw_key"], key)

    def test_reject_path_traversal_and_duplicate_chapters(self):
        self.doc["chapters"][0]["pages"][0]["fixture_file"] = "../outside.html"
        with self.assertRaises(NeedsReview):
            parse_plan(self.data())
        self.doc["chapters"][0]["pages"][0]["fixture_file"] = "chapter1.html"
        self.doc["chapters"].append(self.doc["chapters"][0])
        with self.assertRaises(NeedsReview):
            parse_plan(self.data())

    def test_same_work_different_approval_is_rejected(self):
        self.ledger.register(self.plan)
        self.doc["title"] = "A different approved title"
        with self.assertRaises(NeedsReview):
            self.ledger.register(parse_plan(self.data()))

    def test_invalid_text_and_scripts_are_rejected(self):
        for raw in (b'<article data-fixture-work="work-demo" data-chapter="1" data-page="1"><script>a</script><p>x</p></article>',
                    b'<article data-fixture-work="work-demo" data-chapter="1" data-page="1"></article>',
                    b'<article data-fixture-work="work-demo" data-chapter="1" data-page="2"><p>x</p></article>'):
            with self.subTest(raw=raw), self.assertRaises(NeedsReview):
                parse_snapshot(raw,source_id="work-demo",chapter=1,page=1)


if __name__ == "__main__":
    unittest.main()


class PipelineS3FixtureTest(PipelineFixtureTest):
    """End to end over actual boto3 HTTP, not genuine MinIO.

    Inherits the baseline scenarios and executes them with the S3 fixture.
    """
    @classmethod
    def setUpClass(cls):
        from s3_http_server import S3FixtureServer
        import boto3
        from botocore.client import Config
        from discovery_lab.objects import S3ObjectSink
        cls.server = S3FixtureServer()
        cls.server.__enter__()
        cls.s3client = boto3.client('s3', endpoint_url=cls.server.endpoint,
            aws_access_key_id='fixture', aws_secret_access_key='fixture-secret',
            region_name='us-east-1', config=Config(signature_version='s3v4',
            retries={'max_attempts': 0}, s3={'addressing_style': 'path'},
            connect_timeout=2, read_timeout=2))

    @classmethod
    def tearDownClass(cls):
        cls.server.__exit__(None, None, None)

    def setUp(self):
        super().setUp()
        from discovery_lab.objects import S3ObjectSink
        self.sink = S3ObjectSink(self.s3client, 'fixtures',
                                 prefix='m5-' + self._testMethodName + '/')
        self.pipe = UnifiedPipeline(self.ledger, self.sink, self.fixtures,
                                    self.root / 'stage', multipart_journal=self.root / 'mpu.sqlite3')

    def test_raw_corruption_before_parse_fails_closed(self):
        # Fake server has a direct state map for deliberate corruption injection.
        self.ledger.register(self.plan)
        self.pipe.fetch(self.plan)
        key = self.ledger.rows('work-demo')[0]['raw_key']
        full = ('fixtures', self.sink.prefix + key)
        with self.server.state.lock:
            self.server.state.objects[full] = b'corrupt'
        with self.assertRaises(ObjectConflict):
            self.pipe.parse(self.plan)
        self.assertFalse(any(k[1].startswith(self.sink.prefix + 'works/') for k in self.server.state.objects))

    def test_interruption_raw_put_before_sql_receipt_resumes_without_refetch(self):
        self.ledger.register(self.plan)
        with self.assertRaises(InjectCrash):
            self.pipe.fetch(self.plan, crash_after_raw_put=True)
        with self.ledger.tx() as db:
            db.execute("UPDATE source_pages SET lease_until=0 WHERE work_id='work-demo'")
        result = self.pipe.run(self.plan)
        self.assertEqual(result['status'], 'PUBLISHED')
        rawkeys = [key for (_, key) in self.server.state.objects if key.startswith(self.sink.prefix + 'raw/')]
        self.assertEqual(len(rawkeys), 1)

    def test_complete_two_phase_publication_and_readonly_handoff(self):
        report = self.pipe.run(self.plan)
        self.assertEqual(report['status'], 'PUBLISHED')
        self.assertEqual(self.pipe.publisher.verify_current('work-demo')['revision_id'],report['revision_id'])
        self.assertEqual(self.pipe.run(self.plan)['status'], 'UNCHANGED')
        handoff = json.loads(self.sink.get(report['handoff_key']))
        self.assertTrue(handoff['requires_actor_confirmation'])
        self.assertFalse(handoff['publication_permitted'])

    def test_unrecognized_html_fails_no_current(self):
        (self.fixtures / 'chapter1.html').write_text('<article><p>Unknown schema</p></article>')
        with self.assertRaises(NeedsReview):
            self.pipe.run(self.plan)
        self.assertFalse(any(key.startswith(self.sink.prefix + 'works/')
                             for _, key in self.server.state.objects))
        self.assertEqual(self.ledger.status('work-demo')['states'], {'NEEDS_REVIEW': 1})


class CLIDemoTest(unittest.TestCase):
    def test_full_discovery_to_handoff_demo_and_resume(self):
        import subprocess
        import sys
        with tempfile.TemporaryDirectory() as directory:
            working = Path(directory)
            cmd = [sys.executable, '-m', 'discovery_lab.unified_cli', 'demo',
                   '--state', str(working)]
            root = Path(__file__).resolve().parents[1]
            first = subprocess.run(cmd, cwd=root, capture_output=True, text=True, check=True)
            payload = json.loads(first.stdout)
            self.assertEqual(payload['discovery']['status'], 'STABLE_OBSERVED')
            self.assertEqual(payload['pipeline']['status'], 'PUBLISHED')
            self.assertEqual(payload['pipeline']['pages_verified'], 3)
            second = subprocess.run(cmd, cwd=root, capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(second.stdout)['pipeline']['status'], 'UNCHANGED')
            plan = root / 'examples' / 'approved-work' / 'approved-plan.json'
            status = subprocess.run([sys.executable, '-m', 'discovery_lab.unified_cli', 'status',
                                     '--plan', str(plan), '--state', str(working)],
                                     cwd=root, capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(status.stdout)['states'], {'PARSED': 3})
