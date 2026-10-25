"""Standalone, offline-first pipeline executable. No remote-source crawling."""
import argparse
import json
from pathlib import Path

from .objects import FileObjectSink, S3ObjectSink
from .engine import DiscoveryEngine, DiscoveryStore, MemoryFixtureSource, FixtureResponse
from .model import fixture_json
from .unified_pipeline import PageLedger, UnifiedPipeline, parse_plan


def main():
    ap = argparse.ArgumentParser(description="Approved snapshots -> RAW -> parse -> publish -> handoff")
    ap.add_argument("command", choices=("run", "status", "demo"))
    ap.add_argument("--plan", type=Path)
    ap.add_argument("--state", type=Path, required=True, help="private SQLite, spool, local object store")
    ap.add_argument("--fixtures", type=Path, help="approved local fixture directory; no network")
    ap.add_argument("--stop-after", choices=("fetch", "parse"))
    ap.add_argument("--s3-bucket", help="opt-in S3 bucket, uses ambient AWS credentials")
    ap.add_argument("--s3-endpoint", help="opt-in S3 endpoint")
    ap.add_argument("--s3-prefix", default="unified-lab/")
    a = ap.parse_args()
    if a.command != "demo" and a.plan is None:
        ap.error("--plan is required for run/status")
    sample = Path(__file__).resolve().parent.parent / "examples" / "approved-work"
    planfile = sample / "approved-plan.json" if a.command == "demo" else a.plan
    plan = parse_plan(planfile.read_bytes())
    a.state.mkdir(parents=True, exist_ok=True)
    ledger = PageLedger(a.state / "ingest.sqlite3")
    if a.command == "status":
        print(json.dumps(ledger.status(plan.source_id), ensure_ascii=False, indent=2))
        return
    if a.s3_bucket:
        import boto3
        from botocore.client import Config
        sink = S3ObjectSink(boto3.client("s3", endpoint_url=a.s3_endpoint,
                    config=Config(s3={"addressing_style": "path"})),
                    bucket=a.s3_bucket, prefix=a.s3_prefix)
        journal = a.state / "multipart.sqlite3"
    else:
        sink, journal = FileObjectSink(a.state / "objects"), None
    pipeline = UnifiedPipeline(ledger, sink, a.fixtures or planfile.parent,
                               a.state / "stage", multipart_journal=journal)
    if a.command == "demo":
        # Reuse the existing M4 discovery engine; don't manufacture a new frontier.
        catalog = DiscoveryStore(a.state / "discovery.sqlite3")
        source = MemoryFixtureSource({("*", 1): FixtureResponse(200, fixture_json(
            "author_catalog", 1, 1, [{"kind": "work", "id": plan.source_id,
                                       "title": plan.title, "relations": []}], total=1))})
        runs = []
        for _ in range(2):
            rid = catalog.register("author:approved-fixture", "author_catalog",
                                   "https://fixture.invalid/authors/synthetic/works", max_pages=1)
            report = DiscoveryEngine(catalog, sink, source).drain(rid)
            if report["status"] != "MATCHED_DECLARED":
                raise RuntimeError("synthetic catalog was not complete")
            runs.append(rid)
        proof = catalog.assess_passes(runs)
        if proof["status"] not in ("STABLE_OBSERVED", "RECOVERED_STABLE_OBSERVED"):
            raise RuntimeError("synthetic catalog identity/order not stable")
        result = {"discovery": proof, "pipeline": pipeline.run(plan)}
    else:
        result = pipeline.run(plan, stop_after=a.stop_after)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
