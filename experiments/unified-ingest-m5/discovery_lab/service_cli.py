"""Operate a local M8 API / standalone SQLite worker with no live scraping."""
from __future__ import annotations

import argparse
import json
import os
import signal
import threading
from pathlib import Path

from .service import IngestWorker, JobStore, serve


def main():
    ap = argparse.ArgumentParser(description="M8 selected-snapshot ingest service")
    ap.add_argument("command", choices=("serve", "worker", "once", "submit", "list", "get"))
    ap.add_argument("--state", type=Path, required=True)
    ap.add_argument("--approved-root", type=Path, help="operator-approved immutable HTML input directory")
    ap.add_argument("--plan", type=Path, help="selected-work-v1 JSON (operator submit only)")
    ap.add_argument("--job", help="job ID for get")
    ap.add_argument("--port", type=int, default=8765)
    ap.add_argument("--s3-bucket")
    ap.add_argument("--s3-endpoint")
    ap.add_argument("--s3-prefix", default="unified-lab/")
    ap.add_argument("--no-worker", action="store_true", help="serve API but run worker in separate process")
    opt = ap.parse_args()
    approved = opt.approved_root or (opt.state / "approved")
    store = JobStore(opt.state, approved)
    worker = IngestWorker(store, s3_bucket=opt.s3_bucket,
                          s3_endpoint=opt.s3_endpoint, s3_prefix=opt.s3_prefix)
    if opt.command == "submit":
        if not opt.plan:
            ap.error("--plan is required for operator submission")
        # CLI is operator-owned; server HTTP cannot read arbitrary file paths.
        plan = json.loads(opt.plan.read_bytes())
        result, reused = store.submit({"plan": plan})
        print(json.dumps({"job": result, "reused": reused}, ensure_ascii=False, indent=2))
    elif opt.command == "list":
        print(json.dumps({"jobs": store.list()}, ensure_ascii=False, indent=2))
    elif opt.command == "get":
        if not opt.job:
            ap.error("--job is required")
        print(json.dumps(store.get(opt.job), ensure_ascii=False, indent=2))
    elif opt.command == "once":
        print(json.dumps(worker.run_once(), ensure_ascii=False, indent=2))
    elif opt.command == "worker":
        stop = threading.Event()
        signal.signal(signal.SIGTERM, lambda *_: stop.set())
        signal.signal(signal.SIGINT, lambda *_: stop.set())
        worker.run_loop(stop)
    else:
        token = os.environ.get("INGEST_API_TOKEN", "")
        if not token:
            ap.error("set INGEST_API_TOKEN (at least 24 characters) before starting HTTP API")
        serve(store, token, port=opt.port, worker=None if opt.no_worker else worker)


if __name__ == "__main__":
    main()
