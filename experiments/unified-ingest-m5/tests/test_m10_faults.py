"""S3 acknowledgement reconciliation and explicit recovery tests."""
import hashlib, io, json, shutil, tempfile, unittest
from pathlib import Path
from discovery_lab.objects import S3ObjectSink, ObjectConflict, ObjectWriteUncertain, FileObjectSink
from discovery_lab.service import JobStore, IngestWorker, JobConflict
from discovery_lab.unified_pipeline import PageLedger

EXAMPLE=Path(__file__).resolve().parents[1]/"examples"/"approved-work"
class Conflict(Exception):
    response={"Error":{"Code":"PreconditionFailed"}}
class S3:
    def __init__(self):
        self.data={};self.counts={};self.drop_put=False;self.drop_read=False
    def put_object(self,*,Key,Body,IfNoneMatch=None,IfMatch=None,**k):
        if IfNoneMatch=="*" and Key in self.data:raise Conflict()
        self.data[Key]=bytes(Body);self.counts[Key]=self.counts.get(Key,0)+1
        if self.drop_put:
            self.drop_put=False
            raise ConnectionResetError("accepted but ACK was lost")
    def get_object(self,*,Key,**k):
        if self.drop_read:raise ConnectionResetError("read disconnected")
        return {"Body":io.BytesIO(self.data[Key])}
    def head_object(self,*,Key,**k):
        return {"ETag":'"etag"'}
class M10FaultTests(unittest.TestCase):
    def test_put_lost_ack_exactly_once(self):
        fake=S3();s=S3ObjectSink(fake,"b","p/")
        fake.drop_put=True
        self.assertEqual(s.put_verified("a",b"good"),hashlib.sha256(b"good").hexdigest())
        self.assertEqual(fake.counts["p/a"],1)
        s.put_verified("a",b"good")
        self.assertEqual(fake.counts["p/a"],1)
    def test_immutable_conflict(self):
        fake=S3();s=S3ObjectSink(fake,"b","p/")
        s.put_verified("a",b"good")
        with self.assertRaises(ObjectConflict):s.put_verified("a",b"bad")
        self.assertEqual(fake.counts["p/a"],1)
    def test_lost_ack_with_unavailable_readback_stays_uncertain(self):
        fake=S3();s=S3ObjectSink(fake,"b","p/")
        fake.drop_put=True;fake.drop_read=True
        with self.assertRaises(ObjectWriteUncertain):s.put_verified("a",b"good")
        fake.drop_read=False;s.put_verified("a",b"good")
        self.assertEqual(fake.counts["p/a"],1)
    def test_create_cas_ack_lost(self):
        fake=S3();s=S3ObjectSink(fake,"b","p/")
        fake.drop_put=True
        self.assertEqual(s.cas_verified("current.json",b"new",None),hashlib.sha256(b"new").hexdigest())
        self.assertEqual(fake.counts["p/current.json"],1)
    def test_explicit_job_retry_after_uncertain_raw(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);approved=root/"approved"/"demo-story";approved.mkdir(parents=True)
            for x in EXAMPLE.glob("*.html"):shutil.copy2(x,approved/x.name)
            jobs=JobStore(root/"state",root/"approved")
            job,_=jobs.submit({"plan":json.loads((EXAMPLE/"approved-plan.json").read_text())})
            worker=IngestWorker(jobs)
            class Flaky(FileObjectSink):
                lost=False
                def put_verified(self,key,data):
                    sha=super().put_verified(key,data)
                    if not self.lost:
                        self.lost=True
                        raise ObjectWriteUncertain("ACK lost")
                    return sha
            sink=Flaky(root/"state"/"objects");worker._sink=lambda:sink
            self.assertEqual(worker.run_once()["error_code"],"ObjectWriteUncertain")
            self.assertEqual(PageLedger(jobs.ledger_path(job["job_id"])).status("demo-story")["issues"],[])
            self.assertEqual(jobs.retry(job["job_id"])["status"],"QUEUED")
            self.assertEqual(worker.run_once()["status"],"COMPLETED")
    def test_non_s3_error_not_retryable(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d);approved=root/"approved"/"demo-story";approved.mkdir(parents=True)
            for x in EXAMPLE.glob("*.html"):shutil.copy2(x,approved/x.name)
            jobs=JobStore(root/"state",root/"approved")
            job,_=jobs.submit({"plan":json.loads((EXAMPLE/"approved-plan.json").read_text())})
            worker=IngestWorker(jobs)
            worker._process=lambda *args: (_ for _ in ()).throw(RuntimeError("not transport"))
            self.assertEqual(worker.run_once()["status"],"NEEDS_REVIEW")
            with self.assertRaises(JobConflict):jobs.retry(job["job_id"])
