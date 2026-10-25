"""M9: independent works run concurrently, abandoned jobs recover without a global reset."""
from __future__ import annotations

import concurrent.futures
import json
import shutil
import tempfile
import threading
import time
import unittest
from pathlib import Path

from discovery_lab.service import IngestWorker, JobConflict, JobStore, WorkerBusy

EXAMPLE=Path(__file__).resolve().parents[1]/"examples"/"approved-work"


class M9ConcurrencyTest(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory(prefix="m9-jobs-")
        self.addCleanup(self.tmp.cleanup)
        self.root=Path(self.tmp.name)
        self.state=self.root/"state"
        self.approved=self.root/"approved"
        self.jobs=JobStore(self.state,self.approved)

    def submit_work(self, ident):
        plan=json.loads((EXAMPLE/"approved-plan.json").read_text())
        plan["source_id"]=ident
        plan["source_url"]="https://fixture.invalid/s/"+ident
        target=self.approved/ident
        target.mkdir(parents=True)
        for p in EXAMPLE.glob("*.html"):
            (target/p.name).write_bytes(p.read_bytes().replace(b'data-fixture-work="demo-story"',
                                                             ('data-fixture-work="'+ident+'"').encode()))
        return self.jobs.submit({"plan":plan})[0]

    def test_parallel_different_jobs_with_per_job_locks(self):
        self.submit_work("work-a")
        self.submit_work("work-b")
        barrier=threading.Barrier(2)
        active=0
        peak=0
        guard=threading.Lock()

        class ObservedWorker(IngestWorker):
            def _process(w,info,plan):
                nonlocal active,peak
                with guard:
                    active+=1
                    peak=max(peak,active)
                try:
                    barrier.wait(timeout=8)
                    return super()._process(info,plan)
                finally:
                    with guard:
                        active-=1

        workers=[ObservedWorker(self.jobs,max_workers=2) for _ in range(2)]
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            futures=[pool.submit(w.run_once) for w in workers]
            res=[f.result(timeout=30) for f in futures]
        self.assertEqual(peak,2)
        self.assertEqual([r["status"] for r in res],["COMPLETED","COMPLETED"])
        self.assertEqual(self.jobs.counts()["COMPLETED"],2)
        self.assertEqual(IngestWorker(self.jobs,max_workers=2).run_once(),None)

    def test_reclaim_only_if_the_specific_job_lock_is_free(self):
        work=self.submit_work("work-a")
        worker=IngestWorker(self.jobs,max_workers=2)
        other=IngestWorker(self.jobs,max_workers=2)
        claimed=self.jobs.claim(work["job_id"])
        self.assertIsNotNone(claimed)
        _,_,stale_token=claimed
        with worker._job_exclusive(work["job_id"]):
            self.assertIsNone(other.run_once())
            self.assertEqual(self.jobs.get(work["job_id"])["status"],"RUNNING")
        out=other.run_once()
        self.assertEqual(out["status"],"COMPLETED",out)
        with self.assertRaises(JobConflict):
            self.jobs.finish(work["job_id"],stale_token,result={"forged":True})

    def test_slots_capped_and_configuration_immutable(self):
        w=IngestWorker(self.jobs,max_workers=2)
        w2=IngestWorker(self.jobs,max_workers=2)
        w3=IngestWorker(self.jobs,max_workers=2)
        with w._exclusive():
            with w2._exclusive():
                with self.assertRaises(WorkerBusy):
                    w3.run_once()
        with self.assertRaises(JobConflict):
            IngestWorker(self.jobs,max_workers=3)
        with self.assertRaises(ValueError):
            IngestWorker(self.jobs,max_workers=9)

    def test_metrics_count_without_sensitive_plan(self):
        self.submit_work("work-a")
        result=self.jobs.counts()
        self.assertEqual(result["QUEUED"],1)
        self.assertNotIn("source",str(result))


if __name__=="__main__":
    unittest.main()
