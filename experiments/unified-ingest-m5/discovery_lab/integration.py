"""Synthetic-only M1 discovery -> M2 publish gate. Not a live site pipeline."""
from __future__ import annotations

import hashlib

from .engine import DiscoveryStore
from .objects import ObjectConflict
from .publisher import Work, WorkObjectSink, WorkPublisher, IncompleteWork, PublicationUncertain


def publish_discovered_fixture(store: DiscoveryStore, sink: WorkObjectSink,
                               completed_pass_ids: list[str], work: Work,
                               *, expected_revision: str | None) -> dict:
    """Bind a synthetic work to two matching discovery snapshots before publish.

    Work chapter texts are synthetic input independent of catalog discovery, so
    this gate proves identity/partition evidence only, NOT that chapter TOC is
    exhaustive on a real website.
    """
    assessment = store.assess_passes(completed_pass_ids)
    if assessment["status"] not in ("STABLE_OBSERVED", "RECOVERED_STABLE_OBSERVED"):
        raise IncompleteWork("work discovery partition is not stable")
    latest_id = completed_pass_ids[-1]
    expected_key = "work:" + work.source_id
    with store._conn() as db:
        seen = db.execute("""SELECT COUNT(*) FROM observations o JOIN snapshots s
                         ON s.job_id=o.job_id WHERE o.run_id=? AND o.entity_key=?""",
                          (latest_id, expected_key)).fetchone()[0]
        rows = list(db.execute("""SELECT object_key,sha256,raw_size FROM snapshots
                            WHERE run_id=? ORDER BY page""", (latest_id,)))
    if not seen or not rows:
        raise IncompleteWork("work ID not found in verified discovery evidence")
    # Independently revalidate the raw receipts, not merely the DB status.
    for key, digest, size in rows:
        try:
            raw = sink.get(key)
        except Exception as exc:
            raise PublicationUncertain("discovery raw receipt unavailable") from exc
        if len(raw) != size or hashlib.sha256(raw).hexdigest() != digest:
            raise ObjectConflict("discovery raw receipt is corrupted")
    result = WorkPublisher(sink).publish(work, expected_revision=expected_revision)
    return {**result, "discovery_run_id": latest_id, "discovery_status": assessment["status"],
            "discovery_raw_verified": len(rows), "scope": assessment["scope"]}
