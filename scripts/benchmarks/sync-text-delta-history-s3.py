#!/usr/bin/env python3
"""DELTA-TEXT-02: actual adjacent Git Markdown versions + disposable two-client MinIO A/B.

No Akula V1/V2 wire writes, no user files, no File Gate, no S3 account
outside localhost. Reuses test-only DELTA-TEXT-01 Zstd/splice codec.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid
from urllib.parse import urlparse

SOURCE_PATHS = (
    "docs/performance-roadmap.md",
    "README.ru.md",
    "docs/sync/adr-0006-cross-process-mutation-boundary.md",
    "docs/sync/adr-0005-inprocess-history-files-interlock.md",
    "aquilum-app/native/testdata/markdown-test.md",
)
MAX_BLOB = 2 * 1024 * 1024
PAIR_LIMIT = 22
MIN_NET_SAVING_BYTES = 128
MIN_NET_SAVING_FRACTION = 0.15


def load_codec():
    filename = Path(__file__).with_name("sync-text-delta-ab.py")
    spec = importlib.util.spec_from_file_location("akula_text_delta01", filename)
    if spec is None or spec.loader is None:
        raise RuntimeError("missing DELTA-TEXT-01 benchmark")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def git(repo: Path, *args: str) -> bytes:
    p = subprocess.run(["git", "-C", str(repo), *args],
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                       check=False, timeout=30)
    if p.returncode != 0:
        raise RuntimeError("Git history lookup failed: " +
                           p.stderr.decode(errors="replace")[:200])
    return p.stdout


def pairs(repo: Path, max_per_file: int, count: int):
    """Adjacent real commits modifying the SAME public Markdown path."""
    for path in SOURCE_PATHS:
        commits = git(repo, "log", "--format=%H", "--", path).decode().splitlines()
        found = 0
        for newer, older in zip(commits, commits[1:]):
            try:
                old = git(repo, "show", older + ":" + path)
                new = git(repo, "show", newer + ":" + path)
            except RuntimeError:
                continue
            if (old == new or len(old) < 64 or len(new) < 64
                    or max(len(old), len(new)) > MAX_BLOB):
                continue
            try:
                old.decode("utf-8")
                new.decode("utf-8")
            except UnicodeDecodeError:
                continue
            yield {
                "path": path, "from_commit": older[:12],
                "to_commit": newer[:12], "base": old, "target": new,
            }
            found += 1
            count -= 1
            if count == 0:
                return
            if found >= max_per_file:
                break


def choose_codec(codec, base: bytes, target: bytes, folder: Path):
    candidates = {}
    for mode, name in ((codec.FULL, "full_zstd"),
                       (codec.SPLICE, "splice_zstd"),
                       (codec.ZSTD_PATCH, "zstd_patch")):
        if mode == codec.ZSTD_PATCH and len(base) < 64:
            continue
        wire, encode_ms = codec.encode(mode, base, target, folder)
        restored, decode_ms = codec.decode(wire, base, folder)
        assert restored == target
        candidates[name] = {
            "mode": mode, "wire": wire,
            "encode_ms": round(encode_ms, 3),
            "decode_ms": round(decode_ms, 3),
            "bytes": len(wire),
        }
    full = candidates["full_zstd"]["bytes"]
    smallest = min(candidates, key=lambda k: candidates[k]["bytes"])
    gain = full - candidates[smallest]["bytes"]
    chosen = smallest if (gain >= MIN_NET_SAVING_BYTES
                          and gain >= full * MIN_NET_SAVING_FRACTION) else "full_zstd"
    return candidates, chosen


def hash_bytes(data: bytes) -> str:
    # Test-only digest. Core's immutable V1 blobs continue to use BLAKE3.
    return hashlib.sha256(data).hexdigest()


class S3Counters:
    def __init__(self, client, bucket: str):
        self.client, self.bucket = client, bucket
        self.get = 0
        self.head = 0
        self.bytes = 0
        self.elapsed_ms = 0.0

    def verified_fetch(self, key: str, expected_hash: str) -> bytes:
        started = time.perf_counter()
        desc = self.client.head_object(Bucket=self.bucket, Key=key)
        self.head += 1
        obj = self.client.get_object(Bucket=self.bucket, Key=key)
        self.get += 1
        data = obj["Body"].read(MAX_BLOB + 512)
        obj["Body"].close()
        self.bytes += len(data)
        self.elapsed_ms += (time.perf_counter()-started) * 1000
        if len(data) > MAX_BLOB + 256 or len(data) != desc["ContentLength"]:
            raise RuntimeError("S3 read length mismatch")
        if hash_bytes(data) != expected_hash:
            raise RuntimeError("S3 immutable object hash mismatch")
        return data

    def stats(self):
        return {
            "head": self.head, "get": self.get,
            "payload_bytes": self.bytes,
            "elapsed_ms": round(self.elapsed_ms, 3),
        }


def make_clients(endpoint: str):
    parsed = urlparse(endpoint)
    if parsed.scheme != "http" or parsed.hostname not in ("127.0.0.1", "localhost", "::1"):
        raise SystemExit("S3 writes permitted only to disposable loopback MinIO")
    import boto3
    from botocore.config import Config
    config = Config(signature_version="s3v4",
                    s3={"addressing_style": "path"},
                    retries={"max_attempts": 1, "mode": "standard"},
                    connect_timeout=5, read_timeout=10)
    user = os.environ.get("AWS_ACCESS_KEY_ID")
    password = os.environ.get("AWS_SECRET_ACCESS_KEY")
    if not user or not password:
        raise SystemExit("Disposable MinIO environment credentials required")
    def create():
        session = boto3.session.Session(aws_access_key_id=user,
                                       aws_secret_access_key=password,
                                       region_name="us-east-1")
        return session.client("s3", endpoint_url=endpoint, config=config)
    return create(), create()  # Independent provider clients, no shared cache.


def store(writer, bucket: str, key: str, data: bytes):
    writer.put_object(Bucket=bucket, Key=key, Body=data,
                      ContentLength=len(data), IfNoneMatch="*")


def minio_case(codec, writer, reader, bucket, i, base, target,
               candidates, selected, folder):
    """Measure actual object reads with verified cached base, cold base and full fallback."""
    # These are *experimental hint objects*, not real Akula S3 manifests.
    prefix = "text-delta-02/" + format(i, "03") + "/"
    base_key = prefix + "v1/base-raw-" + hash_bytes(base)
    target_key = prefix + "v1/new-raw-" + hash_bytes(target)
    full = candidates["full_zstd"]["wire"]
    chosen_wire = candidates[selected]["wire"]
    full_key = prefix + "sidecar/full-" + hash_bytes(full)
    selected_key = prefix + "sidecar/selected-" + hash_bytes(chosen_wire)
    for key, payload in ((base_key, base), (target_key, target),
                         (full_key, full), (selected_key, chosen_wire)):
        store(writer, bucket, key, payload)

    def receive(wire_key, expected, verified_base=None, fetch_base=False):
        stats = S3Counters(reader, bucket)
        if fetch_base:
            verified_base = stats.verified_fetch(base_key, hash_bytes(base))
        wire = stats.verified_fetch(wire_key, hash_bytes(expected))
        restored, _ = codec.decode(wire, verified_base, folder)
        if restored != target:
            raise RuntimeError("S3 receiver restored different Markdown bytes")
        return stats.stats()

    # A baseline client needs only the full (compressed) representation.
    full_remote = receive(full_key, full, verified_base=None)
    # Hit: receiver *already* holds an independently verified old revision.
    warm_remote = receive(selected_key, chosen_wire, verified_base=base)
    # Miss: receive exact immutable old bytes before decoding the patch.
    cold_remote = receive(selected_key, chosen_wire, fetch_base=(selected!="full_zstd"))
    # Absence of a pinned old version: must NOT apply delta to an arbitrary
    # replacement. The full compressed representation is fetched instead.
    missing_remote = receive(full_key, full, verified_base=None)
    if selected != "full_zstd":
        codec.must_reject(chosen_wire, target, folder)
        codec.must_reject(chosen_wire, None, folder)
    if full_remote["get"] != 1 or warm_remote["get"] != 1:
        raise AssertionError("unexpected S3 read request accounting")
    if cold_remote["get"] != (1 if selected=="full_zstd" else 2):
        raise AssertionError("cold cache must fetch its verified old blob")

    # V1 *still* uploads the full new raw blob even if the V2 sidecar is
    # a tiny delta. These are observed object payload sizes, not TCP/TLS bytes.
    return {
        "full_baseline": full_remote,
        "warm_cached_old": warm_remote,
        "cold_fetch_old": cold_remote,
        "missing_base_full_fallback": missing_remote,
        "v1_full_new_blob_put_bytes": len(target),
        "sidecar_full_put_bytes": len(full),
        "sidecar_selected_put_bytes": len(chosen_wire),
        "warm_net_saving_payload_bytes": full_remote["payload_bytes"]-
                                         warm_remote["payload_bytes"],
        "cold_net_saving_payload_bytes": full_remote["payload_bytes"]-
                                         cold_remote["payload_bytes"],
        "sidecar_plus_required_v1_full_bytes": len(target)+len(chosen_wire),
        "full_sidecar_plus_required_v1_full_bytes": len(target)+len(full),
    }


def run():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--repo", type=Path, default=Path.cwd())
    p.add_argument("--output", type=Path,
                   default=Path("sync-text-delta-history-s3.jsonl"))
    p.add_argument("--max-per-file", type=int, default=4)
    p.add_argument("--max-pairs", type=int, default=20)
    p.add_argument("--minio-endpoint")
    args = p.parse_args()
    if not 1 <= args.max_per_file <= 8 or not 1 <= args.max_pairs <= PAIR_LIMIT:
        raise SystemExit("pair budget exceeded")
    repo = args.repo.resolve()
    if not (repo/".git").exists():
        raise SystemExit("full Git history required; use checkout fetch-depth: 0")
    codec = load_codec()
    codec.self_test()
    all_pairs = list(pairs(repo, args.max_per_file, args.max_pairs))
    if len(all_pairs) < 9 or len({x["path"] for x in all_pairs}) < 3:
        raise SystemExit("insufficient real adjacent Git revision pairs")

    writer = reader = None
    bucket = None
    if args.minio_endpoint:
        writer, reader = make_clients(args.minio_endpoint)
        bucket = "akula-text-delta-02-" + uuid.uuid4().hex[:16]
        writer.create_bucket(Bucket=bucket)

    records = []
    try:
        with tempfile.TemporaryDirectory(prefix="akula-real-history-delta-") as dirname:
            folder = Path(dirname)
            for idx, pair in enumerate(all_pairs):
                old, new = pair["base"], pair["target"]
                candidates, chosen = choose_codec(codec, old, new, folder)
                modes = {
                    name: {k:v for k,v in value.items() if k not in ("wire","mode")}
                    for name,value in candidates.items()
                }
                record = {
                    "kind": "actual_adjacent_git_commits",
                    "public_file": pair["path"], "from_commit":pair["from_commit"],
                    "to_commit": pair["to_commit"], "old_bytes":len(old),
                    "new_bytes":len(new), "old_sha256":hash_bytes(old),
                    "new_sha256":hash_bytes(new), "modes":modes,
                    "selected":chosen,
                    "full_wire_bytes":candidates["full_zstd"]["bytes"],
                    "selected_wire_bytes":candidates[chosen]["bytes"],
                    "notes":"no production V1/V2 format, source code or files altered",
                }
                if bucket:
                    record["real_two_client_minio"] = minio_case(
                        codec, writer, reader, bucket, idx, old, new,
                        candidates, chosen, folder)
                records.append(record)
                print("DELTA_REAL_PAIR " + json.dumps({
                    "path":record["public_file"], "commits":[record["from_commit"],record["to_commit"]],
                    "bytes":record["new_bytes"],"full":record["full_wire_bytes"],
                    "selected":record["selected"],"wire":record["selected_wire_bytes"],
                    "warm_get":record.get("real_two_client_minio",{}).get("warm_cached_old",{}).get("get"),
                    "cold_get":record.get("real_two_client_minio",{}).get("cold_fetch_old",{}).get("get"),
                }, ensure_ascii=False, sort_keys=True))
            args.output.parent.mkdir(parents=True,exist_ok=True)
            args.output.write_text(
                "".join(json.dumps(x,sort_keys=True,ensure_ascii=False)+"\n" for x in records),
                encoding="utf-8")
    finally:
        if bucket:
            # Isolated bucket was freshly created by this exact invocation.
            # No other buckets, manifests, credentials or files are touched.
            paginator = writer.get_paginator("list_objects_v2")
            for page in paginator.paginate(Bucket=bucket):
                for obj in page.get("Contents",[]):
                    writer.delete_object(Bucket=bucket, Key=obj["Key"])
            writer.delete_bucket(Bucket=bucket)

    full=sum(x["full_wire_bytes"] for x in records)
    chosen=sum(x["selected_wire_bytes"] for x in records)
    total_raw=sum(x["new_bytes"] for x in records)
    stats = {
        "pairs":len(records), "distinct_files":len({x["public_file"] for x in records}),
        "selected_counts":{name:sum(x["selected"]==name for x in records)
                           for name in ("full_zstd","splice_zstd","zstd_patch")},
        "sidecar_full_bytes":full, "sidecar_selected_bytes":chosen,
        "sidecar_saving_pct":round((full-chosen)/full*100,2),
        "required_full_v1_blob_bytes":total_raw,
        "counterfactual_sender_full_plus_v1":full+total_raw,
        "counterfactual_sender_selected_plus_v1":chosen+total_raw,
        "counterfactual_sender_saving_pct":round(
            (full-chosen)/(full+total_raw)*100,2),
        "uses_real_two_client_minio":bool(bucket),
        "assurance":"test-only-public-git-history-no-V1-V2-write-no-user-File-Gate",
        "limits":"no HTTP/TLS header billing, no cold V1 DAG full pin, no production release claim",
    }
    if bucket:
        remote=[x["real_two_client_minio"] for x in records]
        stats["actual_minio_read_payload_bytes"]={
            mode:sum(x[mode]["payload_bytes"] for x in remote)
            for mode in ("full_baseline","warm_cached_old","cold_fetch_old",
                         "missing_base_full_fallback")
        }
        stats["actual_minio_get_requests"]={
            mode:sum(x[mode]["get"] for x in remote)
            for mode in ("full_baseline","warm_cached_old","cold_fetch_old",
                         "missing_base_full_fallback")
        }
        stats["actual_minio_head_requests"]={
            mode:sum(x[mode]["head"] for x in remote)
            for mode in ("full_baseline","warm_cached_old","cold_fetch_old",
                         "missing_base_full_fallback")
        }
    print("DELTA_REAL_SUMMARY "+json.dumps(stats,sort_keys=True))


if __name__ == "__main__":
    run()
