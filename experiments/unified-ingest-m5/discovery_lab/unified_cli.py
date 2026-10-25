"""Standalone, offline-first pipeline executable. No remote-source crawling."""
import argparse
import json
from pathlib import Path

from .objects import FileObjectSink, S3ObjectSink
from .engine import DiscoveryEngine, DiscoveryStore, MemoryFixtureSource, FixtureResponse
from .model import fixture_json
from .unified_pipeline import PageLedger, UnifiedPipeline, parse_plan
from .erida_export import prepare_erida_batches
from .erida_preview import preview_erida_batches


def main():
    ap = argparse.ArgumentParser(description="Approved snapshots -> RAW -> parse -> publish -> handoff")
    ap.add_argument("command", choices=("run", "status", "demo", "export-erida", "preview-erida"))
    ap.add_argument("--plan", type=Path)
    ap.add_argument("--state", type=Path, required=True, help="private SQLite, spool, local object store")
    ap.add_argument("--fixtures", type=Path, help="approved local fixture directory; no network")
    ap.add_argument("--stop-after", choices=("fetch", "parse"))
    ap.add_argument("--s3-bucket", help="opt-in S3 bucket, uses ambient AWS credentials")
    ap.add_argument("--s3-endpoint", help="opt-in S3 endpoint")
    ap.add_argument("--s3-prefix", default="unified-lab/")
    ap.add_argument("--erida-out", type=Path, help="write read-only verified Erida HTML import batches")
    ap.add_argument("--handoff-key", help="immutable handoff object key for export-erida")
    ap.add_argument("--handoff-sha256", help="expected immutable handoff SHA-256 for export-erida")
    ap.add_argument("--erida-manifest", type=Path, help="prepared local Erida import manifest for preview-erida")
    ap.add_argument("--erida-origin", help="explicit trusted Erida HTTPS origin for read-only preview")
    a = ap.parse_args()
    if a.command in ("run", "status") and a.plan is None:
        ap.error("--plan is required for run/status")
    if a.command == "export-erida" and (not a.erida_out or not a.handoff_key or not a.handoff_sha256):
        ap.error("export-erida requires --erida-out, --handoff-key and --handoff-sha256")
    if a.command == "preview-erida":
        if not a.erida_manifest or not a.erida_origin:
            ap.error("preview-erida requires --erida-manifest and --erida-origin")
        import os
        key = os.environ.get("ERIDA_API_KEY", "")
        if not key:
            ap.error("explicit authenticated preview requires ERIDA_API_KEY environment variable")
        print(json.dumps(preview_erida_batches(a.erida_manifest, erida_origin=a.erida_origin, api_key=key),
                         ensure_ascii=False, indent=2))
        return
    sample = Path(__file__).resolve().parent.parent / "examples" / "approved-work"
    planfile = sample / "approved-plan.json" if a.command == "demo" else a.plan
    plan = None if a.command == "export-erida" else parse_plan(planfile.read_bytes())
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
    if a.command == "export-erida":
        print(json.dumps(prepare_erida_batches(sink, a.handoff_key, a.handoff_sha256, a.erida_out),
                         ensure_ascii=False, indent=2))
        return
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
    if a.erida_out:
        receipt = result["pipeline"] if a.command == "demo" else result
        if not receipt.get("handoff_key") or not receipt.get("handoff_sha256"):
            ap.error("Erida export requires a completed published handoff")
        result["erida_export"] = prepare_erida_batches(sink, receipt["handoff_key"],
                                                       receipt["handoff_sha256"], a.erida_out)
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()

