"""End-to-end Scrapy vs Spider experiment against a real, versioned S3 API.

Requires boto3, psutil, Scrapy (only the Python worker) and a pre-built Rust binary.
No data beyond a synthetic local HTTP site is crawled.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import signal
import socket
import statistics
import subprocess
import sys
import time
import urllib.request
import uuid
from pathlib import Path

import boto3
from botocore.client import Config
from botocore.exceptions import ClientError
import psutil

from fixtures import generate

DIR = Path(__file__).resolve().parent


def client():
    return boto3.client(
        "s3", endpoint_url=os.environ["S3_ENDPOINT"], region_name="us-east-1",
        config=Config(s3={"addressing_style": "path"}, max_pool_connections=10),
    )


def wait_for_minio(seconds=90):
    start = time.monotonic()
    while time.monotonic() - start < seconds:
        try:
            client().list_buckets()
            return
        except Exception:
            time.sleep(1)
    raise TimeoutError("MinIO S3 API did not become available")


def prepare_bucket(bucket):
    c = client()
    try:
        c.head_bucket(Bucket=bucket)
    except ClientError as exc:
        if exc.response["ResponseMetadata"]["HTTPStatusCode"] != 404:
            raise
        c.create_bucket(Bucket=bucket)
    # Object versions allow exact proof of no duplicate acknowledged writes on restart.
    c.put_bucket_versioning(Bucket=bucket, VersioningConfiguration={"Status": "Enabled"})
    state = c.get_bucket_versioning(Bucket=bucket)
    if state.get("Status") != "Enabled":
        raise RuntimeError(f"S3 bucket versioning is not enabled: {state}")


def object_versions(bucket, prefix):
    c = client()
    counts = {}
    for page in c.get_paginator("list_object_versions").paginate(Bucket=bucket, Prefix=prefix + "/"):
        for item in page.get("Versions", []):
            key = item["Key"]
            if key.startswith(prefix + "/"):
                counts[key] = counts.get(key, 0) + 1
        for item in page.get("DeleteMarkers", []):
            key = item["Key"]
            if key.startswith(prefix + "/"):
                raise AssertionError(f"unexpected delete marker: {key}")
    return counts


def verify(bucket, prefix, manifest, complete=True):
    c = client()
    seen = object_versions(bucket, prefix)
    expected = {f"{prefix}/{n}" for n in manifest}
    unknown = set(seen) - expected
    if unknown:
        raise AssertionError(f"unexpected S3 keys: {sorted(unknown)[:5]}")
    if complete and set(seen) != expected:
        missing = expected - set(seen)
        raise AssertionError(f"missing S3 objects: {len(missing)}; sample {sorted(missing)[:5]}")
    if any(count != 1 for count in seen.values()):
        raise AssertionError(f"duplicate S3 versions: {[k for k,v in seen.items() if v != 1][:5]}")
    total_bytes = 0
    for key in seen:
        n = key.rsplit("/", 1)[1]
        response = c.get_object(Bucket=bucket, Key=key)
        try:
            body = response["Body"].read()
        finally:
            response["Body"].close()
        sha = hashlib.sha256(body).hexdigest()
        if sha != manifest[n]["sha256"]:
            raise AssertionError(f"S3 body checksum mismatch: {key}")
        if response.get("Metadata", {}).get("sha256") != sha:
            raise AssertionError(f"S3 metadata checksum mismatch: {key}")
        total_bytes += len(body)
    return {"objects": len(seen), "bytes": total_bytes, "versions": sum(seen.values())}


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_for_http(root, seconds=10):
    for _ in range(seconds * 10):
        try:
            with urllib.request.urlopen(root, timeout=1) as response:
                if response.status == 200:
                    return
        except (OSError, TimeoutError):
            time.sleep(0.1)
    raise TimeoutError("synthetic web server did not start")


def run_process(command, env, logfile: Path, timeout=180):
    logfile.parent.mkdir(parents=True, exist_ok=True)
    with logfile.open("w", encoding="utf-8") as output:
        proc = subprocess.Popen(command, env=env, cwd=DIR, stdout=output, stderr=subprocess.STDOUT,
                                start_new_session=True)
        ps_proc = psutil.Process(proc.pid)
        peak_rss = 0
        sampled_cpu = 0.0
        start = time.monotonic()
        try:
            while proc.poll() is None:
                if time.monotonic() - start > timeout:
                    os.killpg(proc.pid, signal.SIGKILL)
                    raise TimeoutError(f"process timeout: {command}")
                try:
                    peak_rss = max(peak_rss, ps_proc.memory_info().rss)
                    cpu = ps_proc.cpu_times()
                    sampled_cpu = cpu.user + cpu.system
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    pass
                time.sleep(0.02)
            rc = proc.wait()
        finally:
            if proc.poll() is None:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait()
    return {"returncode": rc, "wall_s": round(time.monotonic() - start, 4),
            "sampled_cpu_s": round(sampled_cpu, 3), "sampled_peak_rss_mib": round(peak_rss / 2**20, 2),
            "log": str(logfile)}


def worker(mode, rust_binary, env, log):
    if mode == "python":
        command = [sys.executable, str(DIR / "python_crawler.py")]
    else:
        command = [str(rust_binary)]
    return run_process(command, env, log)


def assert_success(result):
    if result["returncode"] != 0:
        snippet = Path(result["log"]).read_text(encoding="utf-8")[-3000:]
        raise RuntimeError(f"worker failed with code {result['returncode']}:\n{snippet}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("command", choices=["run", "wait", "fixture-only"])
    ap.add_argument("--rust-binary", type=Path, default=DIR / "target/release/rust-spider-s3-trial")
    ap.add_argument("--pages", type=int, default=160)
    ap.add_argument("--size", type=int, default=128*1024)
    ap.add_argument("--rounds", type=int, default=3)
    ap.add_argument("--out", type=Path, default=DIR / "artifacts")
    args = ap.parse_args()
    if args.command == "wait":
        wait_for_minio()
        print("MinIO S3 API ready")
        return
    args.out.mkdir(parents=True, exist_ok=True)
    generated = args.out / "web"
    manifest = generate(generated, args.pages, args.size)
    if args.command == "fixture-only":
        print(f"generated {len(manifest)} pages")
        return
    if not args.rust_binary.is_file():
        ap.error(f"Rust binary not found: {args.rust_binary}; cargo build --release first")
    wait_for_minio()
    bucket = "spider-benchmark"
    prepare_bucket(bucket)
    port = free_port()
    root = f"http://127.0.0.1:{port}/index.html"
    webserver = subprocess.Popen([sys.executable, "-m", "http.server", str(port),
                                  "--bind", "127.0.0.1", "--directory", str(generated)],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    results = {"scope": "real MinIO S3 API, 160 synthetic HTML pages, boto3 vs Spider + aws-sdk-s3",
               "pages": args.pages, "page_size_approx": args.size, "rounds": args.rounds,
               "python": [], "rust": [], "recovery": {}, "environment": {
                   "endpoint": os.environ.get("S3_ENDPOINT", ""),
                   "minio_image": os.environ.get("MINIO_IMAGE", ""),
                   "python": sys.version.split()[0],
                   "ci_run": os.environ.get("GITHUB_RUN_ID", "local")}}
    runid = f"{os.environ.get('GITHUB_RUN_ID', 'local')}-{uuid.uuid4().hex[:8]}"
    try:
        wait_for_http(root)
        common_env = os.environ.copy()
        common_env.update({"TRIAL_ROOT": root, "TRIAL_PAGES": str(args.pages), "S3_BUCKET": bucket})
        # Alternating order limits cache order bias. No build time included.
        for i in range(args.rounds):
            for mode in (("python", "rust") if i % 2 == 0 else ("rust", "python")):
                env = dict(common_env, S3_PREFIX=f"{runid}/clean/{i}/{mode}", CRASH_AFTER_PUT_N="0")
                stats = worker(mode, args.rust_binary, env, args.out / f"clean-{i}-{mode}.log")
                assert_success(stats)
                stats["integrity"] = verify(bucket, env["S3_PREFIX"], manifest)
                results[mode].append(stats)
                print(f"CLEAN {mode} round={i} time={stats['wall_s']}s rss={stats['sampled_peak_rss_mib']}MiB", flush=True)
        for mode in ("python", "rust"):
            env = dict(common_env, S3_PREFIX=f"{runid}/recovery/{mode}", CRASH_AFTER_PUT_N="18")
            first = worker(mode, args.rust_binary, env, args.out / f"crash-{mode}.log")
            if first["returncode"] != 87:
                raise AssertionError(f"expected crash after acknowledged S3 PUT, got: {first}")
            partial = verify(bucket, env["S3_PREFIX"], manifest, complete=False)
            if not (0 < partial["objects"] < args.pages):
                raise AssertionError(f"crash produced invalid number of objects: {partial}")
            env["CRASH_AFTER_PUT_N"] = "0"
            restarted = worker(mode, args.rust_binary, env, args.out / f"restart-{mode}.log")
            assert_success(restarted)
            final = verify(bucket, env["S3_PREFIX"], manifest)
            results["recovery"][mode] = {"crashed": first, "partial": partial,
                                          "restart": restarted, "integrity": final,
                                          "duplicate_versions": 0}
            print(f"RECOVER {mode} before={partial['objects']} after={final['objects']} versions={final['versions']}", flush=True)
        results["summary"] = {mode: {
            "median_wall_s": round(statistics.median(x["wall_s"] for x in results[mode]), 4),
            "median_sampled_rss_mib": round(statistics.median(x["sampled_peak_rss_mib"] for x in results[mode]), 2),
            "median_pages_per_s": round(args.pages / statistics.median(x["wall_s"] for x in results[mode]), 2),
        } for mode in ("python", "rust")}
        results["status"] = "PASSED"
        print(json.dumps(results["summary"], indent=2), flush=True)
    finally:
        if webserver.poll() is None:
            os.killpg(webserver.pid, signal.SIGTERM)
            webserver.wait(timeout=5)
        (args.out / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"Results written to {args.out / 'results.json'}", flush=True)


if __name__ == "__main__":
    main()
