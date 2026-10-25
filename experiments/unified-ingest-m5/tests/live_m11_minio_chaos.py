"""M11 acceptance: real TCP disconnects, actual MinIO restart, durable multipart resume.

Runs ONLY against an explicitly opted-in disposable local MinIO container.
No web crawling, no Erida writes, no secrets or raw content in the report.
"""
from __future__ import annotations

import hashlib
import io
import json
import os
import shutil
import subprocess
import tempfile
import time
import uuid
from pathlib import Path
from urllib.parse import urlsplit

import boto3
from botocore.config import Config
from discovery_lab.objects import ObjectWriteUncertain, S3ObjectSink
from discovery_lab.multipart import MiB, MultipartFileUploader
from discovery_lab.service import JobStore, IngestWorker
from discovery_lab.streaming_publisher import StreamingWorkPublisher
from m11_fault_proxy import FaultProxy

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "examples" / "approved-work"
if os.environ.get("M11_ALLOW_CHAOS") != "local-disposable-minio":
    raise SystemExit("M11 chaos not explicitly authorized")
BACKEND = os.environ.get("S3_ENDPOINT", "")
U = urlsplit(BACKEND)
if U.scheme != "http" or U.hostname != "127.0.0.1" or U.port is None:
    raise SystemExit("M11 requires explicit loopback-only MinIO endpoint")
CONTAINER = os.environ.get("M11_MINIO_CONTAINER", "")
if CONTAINER != "ingest-m11-minio":
    raise SystemExit("M11 requires named disposable MinIO container")
BUCKET = "unified-m11-ci"
PREFIX = "m11/" + os.environ.get("GITHUB_RUN_ID", "local") + "/" + uuid.uuid4().hex[:12] + "/"


def client(endpoint: str, *, quick: bool = True):
    return boto3.client("s3", endpoint_url=endpoint, region_name="us-east-1",
        config=Config(s3={"addressing_style": "path"},
                      retries={"total_max_attempts": 1 if quick else 3},
                      connect_timeout=2, read_timeout=4))


def wait_ready(timeout=45):
    c = client(BACKEND)
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            c.list_buckets()
            return
        except Exception:
            time.sleep(.4)
    raise AssertionError("MinIO did not recover within allowed time")


def restart_minio():
    """Actual Docker stop/start: persists /data within the same disposable container."""
    subprocess.run(["docker", "stop", "--time", "3", CONTAINER], check=True,
                   stdout=subprocess.DEVNULL)
    try:
        # Probe real downtime; no successful socket can reach S3 at the mapped port.
        try:
            client(BACKEND).list_buckets()
        except Exception:
            pass
        else:
            raise AssertionError("MinIO expected to be offline")
    finally:
        subprocess.run(["docker", "start", CONTAINER], check=True, stdout=subprocess.DEVNULL)
    wait_ready()


def versions(c, key):
    out = c.list_object_versions(Bucket=BUCKET, Prefix=key)
    return [v for v in out.get("Versions", ()) if v["Key"] == key]


def digest_remote(c, key):
    stream = c.get_object(Bucket=BUCKET, Key=key)["Body"]
    h, length = hashlib.sha256(), 0
    try:
        while block := stream.read(256 * 1024):
            h.update(block)
            length += len(block)
    finally:
        stream.close()
    return h.hexdigest(), length


def key(x):
    return PREFIX + x


def fixed_file(path: Path, size: int):
    seed = hashlib.sha256(b"M11 deterministic replayable file").digest() * 8192
    with path.open("wb") as fd:
        left = size
        while left:
            block = seed[:min(len(seed), left)]
            fd.write(block)
            left -= len(block)


def network_failures(s3):
    evidence = {}
    with FaultProxy(BACKEND) as proxy:
        sink = S3ObjectSink(client(proxy.endpoint), BUCKET, PREFIX)
        # First scenario: the backend acknowledged a durable write to the proxy,
        # but no response reached boto3. A read-back must settle the outcome.
        f = proxy.arm("PUT", "ack-lost.html", "after_200")
        body = b"<html><article>durable RAW</article></html>"
        sha = hashlib.sha256(body).hexdigest()
        assert sink.put_verified("raw/ack-lost.html", body) == sha
        assert f.used and len(versions(s3, key("raw/ack-lost.html"))) == 1
        assert digest_remote(s3, key("raw/ack-lost.html")) == (sha, len(body))
        assert sink.put_verified("raw/ack-lost.html", body) == sha
        assert len(versions(s3, key("raw/ack-lost.html"))) == 1
        evidence["ack_lost_after_commit"] = {"verified": True, "versions": 1}

        # Second scenario: the TCP socket closes before any request reaches MinIO.
        f = proxy.arm("PUT", "ack-before.html", "before_send")
        try:
            sink.put_verified("raw/ack-before.html", b"uncommitted")
        except ObjectWriteUncertain:
            pass
        else:
            raise AssertionError("missing PUT must stay uncertain")
        assert f.used
        assert len(versions(s3, key("raw/ack-before.html"))) == 0
        assert sink.put_verified("raw/ack-before.html", b"uncommitted")
        assert len(versions(s3, key("raw/ack-before.html"))) == 1
        evidence["disconnect_before_commit"] = {"uncertain": True, "recovered": True, "versions": 1}

        # Third scenario: truncate GET mid-body; no silent partial acceptance.
        long_body = b"Q" * (256 * 1024)
        sink.put_verified("raw/read-cut.html", long_body)
        f = proxy.arm("GET", "read-cut.html", "partial_get")
        try:
            sink.get("raw/read-cut.html")
        except ObjectWriteUncertain:
            pass
        else:
            raise AssertionError("truncated GET accepted as a complete object")
        assert f.used
        assert sink.get("raw/read-cut.html") == long_body
        evidence["truncated_read"] = {"rejected": True, "readback_recovered": True}

        # Fourth scenario: pointer CAS completed but its acknowledgement vanished.
        f = proxy.arm("PUT", "current.json", "after_200")
        revision = b'{"revision_id":"m11-test","complete":true}'
        assert sink.cas_verified("works/m11/current.json", revision, None) == hashlib.sha256(revision).hexdigest()
        assert f.used and len(versions(s3, key("works/m11/current.json"))) == 1
        evidence["cas_ack_lost"] = {"verified": True, "versions": 1}
    return evidence


def multipart_resume(s3, root: Path):
    evidence = {}
    source = root / "multipart-replayable.bin"
    fixed_file(source, 12 * MiB + 73)
    expected = hashlib.sha256(source.read_bytes()).hexdigest()
    with FaultProxy(BACKEND) as proxy:
        sink = S3ObjectSink(client(proxy.endpoint), BUCKET, PREFIX)
        uploader = MultipartFileUploader(sink, root / "resume.sqlite3", part_bytes=5 * MiB)
        f = proxy.arm("PUT", "partNumber=1", "after_200")
        try:
            uploader.upload_file("archives/resume.bin", source)
        except ObjectWriteUncertain:
            pass
        else:
            raise AssertionError("dropped multipart part ACK did not surface uncertainty")
        assert f.used and uploader.pending(), "upload journal lost its durable UploadId"
        # The state file survives the server restart and a new client/worker.
        restart_minio()
        s3 = client(BACKEND)
        sink = S3ObjectSink(client(proxy.endpoint), BUCKET, PREFIX)
        uploader = MultipartFileUploader(sink, root / "resume.sqlite3", part_bytes=5 * MiB)
        result = uploader.upload_file("archives/resume.bin", source)
        assert result["status"] == "RESUMED", result
        assert result["parts"] == 3, result
        assert digest_remote(s3, key("archives/resume.bin")) == (expected, source.stat().st_size)
        assert len(versions(s3, key("archives/resume.bin"))) == 1
        assert not uploader.pending()
        evidence["resumed_multipart_after_restart"] = {"parts": 3, "verified": True,
                                                        "versions": 1, "size": source.stat().st_size}

        # CompleteMultipartUpload can commit but lose its final response as well.
        other = root / "multipart-completion.bin"
        fixed_file(other, 10 * MiB + 89)
        digest = hashlib.sha256(other.read_bytes()).hexdigest()
        other_uploader = MultipartFileUploader(sink, root / "complete.sqlite3", part_bytes=5 * MiB)
        f = proxy.arm("POST", "uploadId=", "after_200")
        outcome = other_uploader.upload_file("archives/completion.bin", other)
        assert f.used and outcome["status"] in ("PUBLISHED", "RESUMED"), outcome
        assert digest_remote(s3, key("archives/completion.bin")) == (digest, other.stat().st_size)
        assert len(versions(s3, key("archives/completion.bin"))) == 1
        evidence["completion_ack_lost"] = {"verified": True, "versions": 1,
                                            "size": other.stat().st_size}
    return evidence


def job_after_real_outage(s3, root: Path):
    approved = root / "approved" / "m11-restart-story"
    approved.mkdir(parents=True)
    for p in EXAMPLE.glob("*.html"):
        approved.joinpath(p.name).write_bytes(
            p.read_bytes().replace(b'data-fixture-work="demo-story"',
                                   b'data-fixture-work="m11-restart-story"'))
    plan = json.loads((EXAMPLE / "approved-plan.json").read_text())
    plan["source_id"] = "m11-restart-story"
    plan["source_url"] = "https://fixture.invalid/s/m11-restart-story"
    store = JobStore(root / "service-state", root / "approved")
    job, reused = store.submit({"plan": plan, "idempotency_key": "m11-restart"})
    assert not reused
    worker = IngestWorker(store, max_workers=1, s3_bucket=BUCKET, s3_prefix=PREFIX)
    # Override only the transport to enforce short CI retry timeouts.
    worker._sink = lambda: S3ObjectSink(client(BACKEND), BUCKET, PREFIX)
    subprocess.run(["docker", "stop", "--time", "3", CONTAINER], check=True,
                   stdout=subprocess.DEVNULL)
    try:
        failed = worker.run_once()
        assert failed["status"] == "NEEDS_REVIEW" and failed["error_code"] == "ObjectWriteUncertain", failed
    finally:
        subprocess.run(["docker", "start", CONTAINER], check=True, stdout=subprocess.DEVNULL)
        wait_ready()
    assert store.retry(job["job_id"])["status"] == "QUEUED"
    finished = worker.run_once()
    assert finished["status"] == "COMPLETED" and finished["result"]["pages_verified"] == 3, finished
    pub = StreamingWorkPublisher(S3ObjectSink(client(BACKEND), BUCKET, PREFIX),
                                 multipart_journal=root / "publisher.sqlite3")
    verified = pub.verify_current(plan["source_id"])
    assert verified["status"] == "VERIFIED" and verified["files_verified"] >= 4, verified
    return {"first_attempt": "NEEDS_REVIEW", "retry": "COMPLETED", "pages": 3,
            "current_manifest_verified": True}


def main():
    wait_ready()
    s3 = client(BACKEND)
    try:
        s3.create_bucket(Bucket=BUCKET)
    except s3.exceptions.BucketAlreadyOwnedByYou:
        pass
    s3.put_bucket_versioning(Bucket=BUCKET, VersioningConfiguration={"Status": "Enabled"})
    with tempfile.TemporaryDirectory(prefix="m11-real-network-") as d:
        root = Path(d)
        evidence = {"schema": "m11-actual-network-failures-v1", "status": "RUNNING",
                    "source": "synthetic-only", "minio": "real-docker", "scenarios": {}}
        try:
            evidence["scenarios"].update(network_failures(s3))
            evidence["scenarios"].update(multipart_resume(s3, root))
            evidence["scenarios"]["job_after_minio_outage"] = job_after_real_outage(s3, root)
            evidence["status"] = "PASSED"
        except BaseException as exc:
            evidence["status"] = "FAILED"
            evidence["failed_exception_type"] = type(exc).__name__
            raise
        finally:
            out = ROOT / "m11-network-results.json"
            out.write_text(json.dumps(evidence, sort_keys=True, indent=2))
            print(json.dumps(evidence, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()

