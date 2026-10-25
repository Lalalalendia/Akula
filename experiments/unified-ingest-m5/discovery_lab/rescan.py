"""Bounded offline multi-pass reconciliation; fixtures only, zero HTTP support."""
from __future__ import annotations

from collections.abc import Mapping, Sequence

from .engine import DiscoveryEngine, DiscoveryStore, FixtureResponse, MemoryFixtureSource


def verify_partition_across_passes(
    store: DiscoveryStore,
    sink: object,
    fixture_passes: Sequence[Mapping[tuple[str, int], FixtureResponse]],
    *,
    partition: str,
    document_kind: str,
    seed_url: str,
    max_pages: int=200,
    start_clock: float=10_000.0,
) -> dict:
    """Retry entire observed partition and stop on two consecutive clean matches.

    The input passes are *different synthetic snapshots*. In production the
    network side would be a separately admitted/reviewed source, not here.
    A run with more than 8 passes is deliberately unsupported in this lab.
    """
    if not 2 <= len(fixture_passes) <= 8:
        raise ValueError("supply two to eight independent fixture passes")
    run_ids: list[str] = []
    reports: list[dict] = []
    for index, source in enumerate(fixture_passes):
        run_id=store.register(partition,document_kind,seed_url,max_pages=max_pages,
                              when=start_clock+index*3600)
        result=DiscoveryEngine(store,sink,MemoryFixtureSource(source)).drain(
            run_id,start=start_clock+index*3600)
        run_ids.append(run_id)
        reports.append(result)
        if len(run_ids)>=2:
            assessment=store.assess_passes(run_ids)
            if assessment["status"] in ("STABLE_OBSERVED", "RECOVERED_STABLE_OBSERVED"):
                return {"assessment":assessment, "runs":reports, "stopped_early":index+1<len(fixture_passes)}
    return {"assessment":store.assess_passes(run_ids), "runs":reports, "stopped_early":False}
