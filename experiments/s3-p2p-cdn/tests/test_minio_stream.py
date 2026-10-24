#!/usr/bin/env python3
"""Isolated, real MinIO Range/streaming contract for public S3 origin experiments.

Only creates a uniquely named throwaway bucket on the explicitly supplied loopback
endpoint. It never connects to a user's bucket or uses production credentials.
"""
import hashlib
import json
import os
from pathlib import Path
import tempfile
import uuid

import boto3
from boto3.s3.transfer import TransferConfig
from botocore.config import Config
from botocore.exceptions import ClientError

SIZE = 64 * 1024 * 1024
RANGE_SIZE = 256 * 1024
STEP = bytes(i % 251 for i in range(64 * 1024))


def client():
    endpoint = os.environ.get("AQ_S3_TEST_ENDPOINT", "")
    if not endpoint.startswith("http://127.0.0.1:"):
        raise RuntimeError("Refusing MinIO test outside explicit localhost endpoint")
    return boto3.client(
        "s3", endpoint_url=endpoint,
        region_name="us-east-1",
        aws_access_key_id=os.environ["AWS_ACCESS_KEY_ID"],
        aws_secret_access_key=os.environ["AWS_SECRET_ACCESS_KEY"],
        config=Config(s3={"addressing_style": "path"}, retries={"max_attempts": 2}),
    )


def reject(code, operation):
    try:
        operation()
    except ClientError as exc:
        actual = exc.response.get("ResponseMetadata", {}).get("HTTPStatusCode")
        if actual != code:
            raise AssertionError(f"Expected HTTP {code}, received {actual}") from exc
    else:
        raise AssertionError(f"Expected an HTTP {code} rejection")


def consume(body, expected_sha, expected_count):
    digest = hashlib.sha256()
    read = 0
    largest = 0
    try:
        for part in body.iter_chunks(chunk_size=64 * 1024):
            assert len(part) <= 64 * 1024, "S3 consumer buffer exceeded budget"
            largest = max(largest, len(part))
            read += len(part)
            if read > expected_count:
                raise AssertionError("S3 returned excess bytes")
            digest.update(part)
    finally:
        body.close()
    assert read == expected_count, (read, expected_count)
    assert digest.hexdigest() == expected_sha, "S3 byte stream was modified"
    return largest


def run():
    s3 = client()
    bucket = "akula-stream-ci-" + uuid.uuid4().hex[:16]
    key = "immutable/64m.dat"
    get_count = 0
    highest_buffer = 0
    with tempfile.TemporaryDirectory(prefix="akula-s3-ci-") as folder:
        datafile = Path(folder) / "64m.dat"
        digest = hashlib.sha256()
        with datafile.open("wb") as f:
            for _ in range(SIZE // len(STEP)):
                f.write(STEP)
                digest.update(STEP)
        s3.create_bucket(Bucket=bucket)
        try:
            s3.upload_file(
                str(datafile), bucket, key,
                Config=TransferConfig(multipart_threshold=8 * 1024 * 1024,
                                      multipart_chunksize=8 * 1024 * 1024),
            )
            head = s3.head_object(Bucket=bucket, Key=key)
            assert head["ContentLength"] == SIZE
            etag = head["ETag"]  # S3 version validator, never a SHA-256 substitute.

            for start in (0, SIZE // 2, SIZE - RANGE_SIZE):
                end = start + RANGE_SIZE - 1
                with datafile.open("rb") as f:
                    f.seek(start)
                    expected = hashlib.sha256(f.read(RANGE_SIZE)).hexdigest()
                response = s3.get_object(
                    Bucket=bucket, Key=key, Range=f"bytes={start}-{end}", IfMatch=etag,
                )
                get_count += 1
                assert response["ResponseMetadata"]["HTTPStatusCode"] == 206
                assert response["ContentRange"] == f"bytes {start}-{end}/{SIZE}"
                assert response["ContentLength"] == RANGE_SIZE
                highest_buffer = max(highest_buffer, consume(response["Body"], expected, RANGE_SIZE))

            reject(412, lambda: s3.get_object(
                Bucket=bucket, Key=key,
                Range="bytes=0-10", IfMatch='"stale-version"',
            ))
            reject(416, lambda: s3.get_object(
                Bucket=bucket, Key=key, Range=f"bytes={SIZE}-{SIZE + 7}",
            ))

            full = s3.get_object(Bucket=bucket, Key=key, IfMatch=etag)
            get_count += 1
            assert full["ResponseMetadata"]["HTTPStatusCode"] == 200
            highest_buffer = max(highest_buffer, consume(full["Body"], digest.hexdigest(), SIZE))
            result = {
                "suite": "REAL_MINIO_S3_STREAMING_64_MIB",
                "range_gets": 3,
                "full_gets": 1,
                "get_count": get_count,
                "range_bytes_verified": 3 * RANGE_SIZE,
                "full_bytes_verified": SIZE,
                "max_consumer_chunk_bytes": highest_buffer,
                "etag_is_not_sha256": etag.strip('"') != digest.hexdigest(),
                "stale_if_match_rejected": True,
                "out_of_bounds_range_rejected": True,
                "sha256_matches_local_source": True,
                "disposable_localhost_only": True,
            }
            out = Path("test-results/minio-stream-result.json")
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
            print(json.dumps(result, indent=2))
        finally:
            s3.delete_object(Bucket=bucket, Key=key)
            s3.delete_bucket(Bucket=bucket)


if __name__ == "__main__":
    run()
