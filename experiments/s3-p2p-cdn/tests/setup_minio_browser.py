#!/usr/bin/env python3
"""Publish a disposable public-read synthetic object to localhost MinIO.

The only accepted S3 endpoint is the fixed loopback endpoint. Never sends
credentials or real user data off the runner. Produces a pinned SHA manifest.
"""
import hashlib
import json
import os
from pathlib import Path
import urllib.request
import uuid

import boto3
from botocore.config import Config

from create_manifest import make_manifest
from server import ensure_demo

ENDPOINT = "http://127.0.0.1:9000"
PUBLIC_HTTPS = "https://127.0.0.1:9443"
SIZE = 8 * 1024 * 1024


def run():
    if os.environ.get("AQ_S3_TEST_ENDPOINT") != ENDPOINT:
        raise RuntimeError("S3 test endpoint MUST equal fixed localhost endpoint")
    demo = Path("data/demo.bin")
    ensure_demo(demo)
    if demo.stat().st_size != SIZE:
        raise RuntimeError("Unexpected synthetic fixture length")
    key = "immutable/8m.dat"
    bucket = "akula-public-webrtc-" + uuid.uuid4().hex[:14]
    s3 = boto3.client(
        "s3", endpoint_url=ENDPOINT, region_name="us-east-1",
        aws_access_key_id=os.environ["AWS_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["AWS_SECRET_ACCESS_KEY"],
        config=Config(s3={"addressing_style": "path"}, retries={"max_attempts": 2}),
    )
    s3.create_bucket(Bucket=bucket)
    with demo.open("rb") as f:
        s3.put_object(Bucket=bucket, Key=key, Body=f,
                      ContentLength=SIZE, ContentType="application/octet-stream",
                      CacheControl="no-store")
    policy = {
        "Version": "2012-10-17", "Statement": [{
            "Effect": "Allow", "Principal": "*", "Action": ["s3:GetObject"],
            "Resource": [f"arn:aws:s3:::{bucket}/*"],
        }],
    }
    s3.put_bucket_policy(Bucket=bucket, Policy=json.dumps(policy))

    # Prove anonymous read and correct Range response before browser testing.
    first_range = f"{ENDPOINT}/{bucket}/{key}"
    req = urllib.request.Request(first_range, headers={"Range": "bytes=0-262143"})
    with urllib.request.urlopen(req, timeout=10) as resp:
        body = resp.read()
        assert resp.status == 206
        assert resp.headers["Content-Range"] == f"bytes 0-262143/{SIZE}"
        assert len(body) == 262144
        with demo.open("rb") as f:
            assert hashlib.sha256(body).digest() == hashlib.sha256(f.read(262144)).digest()

    manifest = make_manifest(demo, f"{PUBLIC_HTTPS}/{bucket}/{key}")
    assert manifest["size"] == SIZE and len(manifest["chunks"]) == 32
    Path("manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    out = Path("test-results")
    out.mkdir(exist_ok=True)
    (out / "minio-browser-config.json").write_text(json.dumps({
        "bucket": bucket, "key": key, "size": SIZE,
        "content_id": manifest["content_id"],
        "source": "disposable MinIO object (anonymous public GET)",
        "anonymous_range_206": True, "region": "us-east-1",
    }, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"scenario": "MINIO_PUBLIC_OBJECT_CREATED", "bucket": bucket,
                      "size": SIZE, "anonymous_range_206": True,
                      "content_id": manifest["content_id"]}))


if __name__ == "__main__":
    run()
