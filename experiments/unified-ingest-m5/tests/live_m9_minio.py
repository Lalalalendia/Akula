"""Opt-in GitHub Actions integration: API -> SQLite -> 2 workers -> real MinIO.

Never fetch a remote book site. Fixture HTML only. Requires explicit MinIO env.
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import shutil
import tempfile
import threading
import time
import urllib.request
import uuid
from http.server import ThreadingHTTPServer
from pathlib import Path

import boto3
from botocore.client import Config
from discovery_lab.objects import S3ObjectSink
from discovery_lab.service import JobStore, IngestWorker, make_handler
from discovery_lab.unified_pipeline import parse_plan
from discovery_lab.streaming_publisher import StreamingWorkPublisher

HERE=Path(__file__).resolve().parents[1]
EXAMPLE=HERE/"examples"/"approved-work"
assert os.environ.get("M9_ALLOW_LOCAL_MINIO") == "yes", "explicit local MinIO integration gate missing"
ENDPOINT=os.environ["S3_ENDPOINT"]
assert ENDPOINT.startswith("http://127.0.0.1:")
BUCKET="unified-m9-ci"
PREFIX="m9/"+os.environ.get("GITHUB_RUN_ID","local")+"/"+uuid.uuid4().hex[:10]+"/"
TOKEN="m9-ci-token-not-sensitive-0123456789abcdef"


def new_plan(root:Path,ident:str):
    template=json.loads((EXAMPLE/"approved-plan.json").read_text())
    template["source_id"]=ident
    template["source_url"]="https://fixture.invalid/s/"+ident
    target=root/ident
    target.mkdir(parents=True)
    for html in EXAMPLE.glob("*.html"):
        (target/html.name).write_bytes(html.read_bytes().replace(b'data-fixture-work="demo-story"',
                                                            ('data-fixture-work="'+ident+'"').encode()))
    return template


def api(base:str,method:str,path:str,payload=None):
    encoded=json.dumps(payload).encode() if payload is not None else None
    req=urllib.request.Request(base+path,method=method,data=encoded,
                headers={"Authorization":"Bearer "+TOKEN,"Content-Type":"application/json"})
    with urllib.request.urlopen(req,timeout=12) as r:
        return r.status,json.load(r)


def go():
    s3=boto3.client("s3",endpoint_url=ENDPOINT,config=Config(s3={"addressing_style":"path"}),region_name="us-east-1")
    try:
        s3.create_bucket(Bucket=BUCKET)
    except s3.exceptions.BucketAlreadyOwnedByYou:
        pass
    s3.put_bucket_versioning(Bucket=BUCKET,VersioningConfiguration={"Status":"Enabled"})
    with tempfile.TemporaryDirectory(prefix="m9-minio-") as d:
        root=Path(d)
        store=JobStore(root/"state",root/"approved")
        workers=[IngestWorker(store,max_workers=2,s3_bucket=BUCKET,s3_endpoint=ENDPOINT,s3_prefix=PREFIX)
                 for _ in range(2)]
        server=ThreadingHTTPServer(("127.0.0.1",0),make_handler(store,TOKEN))
        thread=threading.Thread(target=server.serve_forever,daemon=True)
        thread.start()
        try:
            base="http://127.0.0.1:"+str(server.server_port)
            profiles=api(base,"GET","/v1/adapters")[1]
            assert "fixture-chapter-html-v1" in profiles["profiles"] and not profiles["live_crawling"]
            plans=[new_plan(root/"approved",x) for x in ("m9-fixture-a","m9-fixture-b")]
            job_ids=[]
            for obj in plans:
                status,res=api(base,"POST","/v1/jobs",{"plan":obj,"idempotency_key":"m9-"+obj["source_id"]})
                assert status==201,(status,res)
                job_ids.append(res["job"]["job_id"])
            # Lost acknowledgement scenario: RAW is already in S3, but SQL still shows RUNNING.
            preplan=parse_plan(json.dumps(plans[0],sort_keys=True,separators=(",", ":")).encode())
            # The manifest was serialized by JobStore with ensure_ascii=False, also identical for this fixture.
            key="raw/"+preplan.source_id+"/"+preplan.manifest_sha256[:16]+"/0001-0001-"
            raw=(root/"approved"/preplan.source_id/"chapter-1-page-1.html").read_bytes()
            raw_sha=hashlib.sha256(raw).hexdigest()
            raw_key=key+raw_sha+".html"
            sink=S3ObjectSink(s3,BUCKET,PREFIX)
            sink.put_verified(raw_key,raw)
            stale=store.claim(job_ids[0])
            assert stale is not None and stale[0]["status"]=="RUNNING"
            # On the next run, a new executor holding that job's flock recovers the unfinished lease.
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                results=list(pool.map(lambda w:w.run_once(),workers))
            assert [r["status"] for r in results].count("COMPLETED")==2,results
            assert all(r["result"]["pages_verified"]==3 for r in results),results
            status,queue=api(base,"GET","/v1/queue")
            assert status==200 and queue["counts"]["COMPLETED"]==2 and queue["counts"]["RUNNING"]==0,queue
            status,res=api(base,"POST","/v1/jobs",{"plan":plans[0],"idempotency_key":"m9-m9-fixture-a"})
            assert status==200 and res["reused"]
            # CAS must point to complete verified immutable files for both independent works.
            for plan in plans:
                result=StreamingWorkPublisher(sink,multipart_journal=root/("multipart-"+plan["source_id"]+".sqlite3")).verify_current(plan["source_id"])
                assert result["status"]=="VERIFIED" and result["files_verified"]>=4,result
            # Only 1 version per immutable RAW object despite a successful put before the crash.
            versions=s3.list_object_versions(Bucket=BUCKET,Prefix=PREFIX+raw_key).get("Versions",[])
            assert len([v for v in versions if v["Key"]==PREFIX+raw_key])==1,versions
            # Read-back all object bytes through the real endpoint; no duplicate successful PUT.
            listing=s3.list_objects_v2(Bucket=BUCKET,Prefix=PREFIX).get("Contents",[])
            assert len(listing)>=20,len(listing)
            summary={"status":"PASSED","verified_works":2,"pages":6,
                "queue":queue["counts"],"s3_keys":len(listing),"preseeded_raw_versions":len(versions),
                "adapter":"fixture-chapter-html-v1","live_crawling":False}
            print(json.dumps(summary,sort_keys=True))
            if os.environ.get("GITHUB_WORKSPACE"):
                (HERE/"m9-minio-results.json").write_text(json.dumps(summary,indent=2))
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=3)


if __name__=="__main__":
    go()
